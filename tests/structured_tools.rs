#![cfg(feature = "libsql")]
//! Integration tests for structured collection tools using file-backed libSQL.
//!
//! Tests the full tool execution path: schema → JSON Schema generation,
//! tool registration, and tool execute() calls against a real database.

use std::sync::Arc;

use serde_json::json;

use ironclaw::context::JobContext;
use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::structured::CollectionSchema;
use ironclaw::tools::builtin::collections::{
    CollectionAddTool, CollectionDeleteTool, CollectionQueryTool, CollectionSummaryTool,
    CollectionUpdateTool, generate_collection_tools,
};
use ironclaw::tools::Tool;

// ==================== Setup ====================

async fn setup() -> (Arc<dyn Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&db_path)
        .await
        .expect("create file-backed db");
    backend.run_migrations().await.expect("run migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, dir)
}

fn test_ctx(user_id: &str) -> JobContext {
    JobContext {
        user_id: user_id.to_string(),
        ..Default::default()
    }
}

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
            "order_count": { "type": "number", "default": 0 },
            "notes": { "type": "text" }
        }
    }))
    .expect("grocery schema should parse")
}

// ==================== Tool Generation ====================

#[tokio::test]
async fn generates_five_tools_per_collection() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();

    let tools = generate_collection_tools(&schema, db);
    assert_eq!(tools.len(), 5);

    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"nanny_shifts_add"));
    assert!(names.contains(&"nanny_shifts_update"));
    assert!(names.contains(&"nanny_shifts_delete"));
    assert!(names.contains(&"nanny_shifts_query"));
    assert!(names.contains(&"nanny_shifts_summary"));
}

#[tokio::test]
async fn add_tool_has_typed_parameters() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();
    let tool = CollectionAddTool::new(schema, Arc::clone(&db));

    let params = tool.parameters_schema();
    // date field should have format: "date"
    assert_eq!(params["properties"]["date"]["format"], "date");
    // start_time should have format: "date-time"
    assert_eq!(params["properties"]["start_time"]["format"], "date-time");
    // hours should be number
    assert_eq!(params["properties"]["hours"]["type"], "number");
    // status should have enum values
    assert!(params["properties"]["status"]["enum"].is_array());
    // required should include date and start_time
    let required = params["required"].as_array().unwrap();
    assert!(required.contains(&json!("date")));
    assert!(required.contains(&json!("start_time")));
}

// ==================== CRUD via Tools ====================

#[tokio::test]
async fn add_and_query_via_tools() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();
    let ctx = test_ctx("andrew");

    // Register the schema first
    db.register_collection("andrew", &schema)
        .await
        .expect("register schema");

    let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db));
    let query_tool = CollectionQueryTool::new(schema, Arc::clone(&db));

    // Add a record via tool
    let result = add_tool
        .execute(
            json!({
                "date": "2026-02-22",
                "start_time": "2026-02-22T08:00:00Z",
                "notes": "Regular shift"
            }),
            &ctx,
        )
        .await
        .expect("add should succeed");

    assert_eq!(result.result["status"], "created");
    let record_id = result.result["record_id"].as_str().unwrap();
    assert!(!record_id.is_empty());

    // Query via tool
    let result = query_tool
        .execute(json!({}), &ctx)
        .await
        .expect("query should succeed");

    assert_eq!(result.result["count"], 1);
    let records = result.result["results"].as_array().unwrap();
    assert_eq!(records[0]["data"]["date"], "2026-02-22");
    assert_eq!(records[0]["data"]["status"], "in_progress"); // default applied
}

#[tokio::test]
async fn add_rejects_invalid_data_via_tool() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();
    let ctx = test_ctx("andrew");

    db.register_collection("andrew", &schema)
        .await
        .expect("register schema");

    let add_tool = CollectionAddTool::new(schema, Arc::clone(&db));

    // Missing required field
    let err = add_tool
        .execute(json!({ "notes": "no date" }), &ctx)
        .await;
    assert!(err.is_err());
}

#[tokio::test]
async fn update_via_tool() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();
    let ctx = test_ctx("andrew");

    db.register_collection("andrew", &schema)
        .await
        .expect("register schema");

    let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db));
    let update_tool = CollectionUpdateTool::new(schema, Arc::clone(&db));

    // Add a record
    let result = add_tool
        .execute(
            json!({
                "date": "2026-02-22",
                "start_time": "2026-02-22T08:00:00Z"
            }),
            &ctx,
        )
        .await
        .unwrap();
    let record_id = result.result["record_id"].as_str().unwrap().to_string();

    // Update via tool
    let result = update_tool
        .execute(
            json!({
                "record_id": record_id,
                "status": "completed",
                "end_time": "2026-02-22T17:00:00Z",
                "hours": 9.0
            }),
            &ctx,
        )
        .await
        .expect("update should succeed");

    assert_eq!(result.result["status"], "updated");

    // Verify the update
    let record = db
        .get_record("andrew", uuid::Uuid::parse_str(&record_id).unwrap())
        .await
        .unwrap();
    assert_eq!(record.data["status"], "completed");
    assert_eq!(record.data["hours"], 9.0);
    assert_eq!(record.data["date"], "2026-02-22"); // original preserved
}

#[tokio::test]
async fn delete_via_tool() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();
    let ctx = test_ctx("andrew");

    db.register_collection("andrew", &schema)
        .await
        .expect("register schema");

    let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db));
    let delete_tool = CollectionDeleteTool::new(schema, Arc::clone(&db));

    // Add and delete
    let result = add_tool
        .execute(
            json!({
                "date": "2026-02-22",
                "start_time": "2026-02-22T08:00:00Z"
            }),
            &ctx,
        )
        .await
        .unwrap();
    let record_id = result.result["record_id"].as_str().unwrap().to_string();

    let result = delete_tool
        .execute(json!({ "record_id": record_id }), &ctx)
        .await
        .expect("delete should succeed");

    assert_eq!(result.result["status"], "deleted");

    // Verify deleted
    assert!(db
        .get_record("andrew", uuid::Uuid::parse_str(&record_id).unwrap())
        .await
        .is_err());
}

// ==================== Query with filters ====================

#[tokio::test]
async fn query_with_filters_via_tool() {
    let (db, _dir) = setup().await;
    let schema = grocery_schema();
    let ctx = test_ctx("andrew");

    db.register_collection("andrew", &schema)
        .await
        .expect("register schema");

    let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db));
    let query_tool = CollectionQueryTool::new(schema, Arc::clone(&db));

    // Add items
    add_tool
        .execute(
            json!({ "name": "milk", "category": "dairy", "on_list": true }),
            &ctx,
        )
        .await
        .unwrap();
    add_tool
        .execute(
            json!({ "name": "bread", "category": "pantry", "on_list": true }),
            &ctx,
        )
        .await
        .unwrap();
    add_tool
        .execute(
            json!({ "name": "eggs", "category": "dairy", "on_list": false }),
            &ctx,
        )
        .await
        .unwrap();

    // Query with filter
    let result = query_tool
        .execute(
            json!({
                "filters": [
                    { "field": "category", "op": "eq", "value": "dairy" }
                ]
            }),
            &ctx,
        )
        .await
        .expect("query should succeed");

    assert_eq!(result.result["count"], 2); // milk and eggs
}

// ==================== Aggregation ====================

#[tokio::test]
async fn summary_sum_via_tool() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();
    let ctx = test_ctx("andrew");

    db.register_collection("andrew", &schema)
        .await
        .expect("register schema");

    let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db));
    let summary_tool = CollectionSummaryTool::new(schema, Arc::clone(&db));

    // Add shifts with hours
    for hours in [8.0, 7.5, 9.0] {
        add_tool
            .execute(
                json!({
                    "date": "2026-02-22",
                    "start_time": "2026-02-22T08:00:00Z",
                    "hours": hours,
                    "status": "completed"
                }),
                &ctx,
            )
            .await
            .unwrap();
    }

    // Sum hours
    let result = summary_tool
        .execute(
            json!({
                "operation": "sum",
                "field": "hours"
            }),
            &ctx,
        )
        .await
        .expect("summary should succeed");

    // The aggregation result is the raw value (e.g., 24.5 for sum)
    let agg = &result.result["aggregation"];
    let total: f64 = agg.as_f64().unwrap_or_else(|| {
        agg.as_str()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0)
    });
    assert!((total - 24.5).abs() < 0.01);
}

#[tokio::test]
async fn summary_count_via_tool() {
    let (db, _dir) = setup().await;
    let schema = grocery_schema();
    let ctx = test_ctx("andrew");

    db.register_collection("andrew", &schema)
        .await
        .expect("register schema");

    let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db));
    let summary_tool = CollectionSummaryTool::new(schema, Arc::clone(&db));

    // Add items
    for name in ["milk", "bread", "eggs", "waffles"] {
        add_tool
            .execute(json!({ "name": name }), &ctx)
            .await
            .unwrap();
    }

    // Count
    let result = summary_tool
        .execute(
            json!({
                "operation": "count",
                "field": "name"
            }),
            &ctx,
        )
        .await
        .expect("count should succeed");

    let agg = &result.result["aggregation"];
    let count: f64 = agg.as_f64().unwrap_or_else(|| {
        agg.as_str()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0)
    });
    assert!((count - 4.0).abs() < 0.01);
}

// ==================== User Isolation via Tools ====================

#[tokio::test]
async fn tools_respect_user_isolation() {
    let (db, _dir) = setup().await;
    let schema = grocery_schema();

    db.register_collection("andrew", &schema)
        .await
        .expect("register for andrew");
    db.register_collection("grace", &schema)
        .await
        .expect("register for grace");

    let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db));
    let query_tool = CollectionQueryTool::new(schema, Arc::clone(&db));

    let andrew_ctx = test_ctx("andrew");
    let grace_ctx = test_ctx("grace");

    // Andrew adds items
    add_tool
        .execute(json!({ "name": "waffles" }), &andrew_ctx)
        .await
        .unwrap();
    add_tool
        .execute(json!({ "name": "milk" }), &andrew_ctx)
        .await
        .unwrap();

    // Grace adds items
    add_tool
        .execute(json!({ "name": "yogurt" }), &grace_ctx)
        .await
        .unwrap();

    // Andrew sees 2 items
    let result = query_tool
        .execute(json!({}), &andrew_ctx)
        .await
        .unwrap();
    assert_eq!(result.result["count"], 2);

    // Grace sees 1 item
    let result = query_tool
        .execute(json!({}), &grace_ctx)
        .await
        .unwrap();
    assert_eq!(result.result["count"], 1);
}
