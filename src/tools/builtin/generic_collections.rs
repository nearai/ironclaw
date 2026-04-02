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

    /// Create a test database (libsql in-memory) for tool struct construction.
    async fn test_db() -> Arc<dyn Database> {
        let (db, _dir) = crate::testing::test_db().await;
        db
    }

    #[tokio::test]
    async fn unified_tool_name_uses_scope() {
        let schema = CollectionSchema {
            collection: "grocery_items".to_string(),
            description: Some("Grocery shopping list".to_string()),
            fields: std::collections::BTreeMap::new(),
            source_scope: None,
        };
        let db = test_db().await;
        let tool = UnifiedCollectionTool::new(schema, db, None, "andrew");
        assert_eq!(tool.name(), "andrew_grocery_items");
    }

    #[tokio::test]
    async fn unified_tool_name_uses_source_scope() {
        let schema = CollectionSchema {
            collection: "tasks".to_string(),
            description: None,
            fields: std::collections::BTreeMap::new(),
            source_scope: Some("household".to_string()),
        };
        let db = test_db().await;
        let tool = UnifiedCollectionTool::new(schema, db, None, "andrew");
        assert_eq!(tool.name(), "household_tasks");
    }

    #[tokio::test]
    async fn unified_tool_schema_has_operation_enum() {
        let schema = CollectionSchema {
            collection: "test".to_string(),
            description: None,
            fields: std::collections::BTreeMap::new(),
            source_scope: None,
        };
        let db = test_db().await;
        let tool = UnifiedCollectionTool::new(schema, db, None, "owner");
        let params = tool.parameters_schema();
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
    async fn generate_unified_returns_one_tool() {
        let schema = CollectionSchema {
            collection: "items".to_string(),
            description: None,
            fields: std::collections::BTreeMap::new(),
            source_scope: None,
        };
        let db = test_db().await;
        let tools = generate_unified_collection_tool(&schema, db, None, "user1");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "user1_items");
    }
}
