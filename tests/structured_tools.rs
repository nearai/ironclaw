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
use ironclaw::tools::{Tool, ToolRegistry};
use ironclaw::tools::builtin::collections::generate_collection_tools;
use ironclaw::tools::builtin::generic_collections::UnifiedCollectionTool;

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
async fn generates_one_unified_tool_per_collection() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();

    let tools = generate_collection_tools(&schema, db, None, "andrew");
    assert_eq!(tools.len(), 1, "unified mode: 1 tool per collection");
    assert_eq!(tools[0].name(), "andrew_nanny_shifts");
}

#[tokio::test]
async fn unified_tool_has_operation_parameter() {
    let (db, _dir) = setup().await;
    let schema = nanny_schema();
    let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

    let params = tool.parameters_schema();
    // operation is required
    let required = params["required"].as_array().unwrap();
    assert!(required.contains(&json!("operation")));
    // operation enum has all 5 ops
    let ops = params["properties"]["operation"]["enum"].as_array().unwrap();
    assert!(ops.contains(&json!("add")));
    assert!(ops.contains(&json!("update")));
    assert!(ops.contains(&json!("delete")));
    assert!(ops.contains(&json!("query")));
    assert!(ops.contains(&json!("summary")));
    // data object for add/update is present
    assert_eq!(params["properties"]["data"]["type"], "object");
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

    let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

    // Add a record via tool
    let result = tool
        .execute(
            json!({
                "operation": "add",
                "data": {
                    "date": "2026-02-22",
                    "start_time": "2026-02-22T08:00:00Z",
                    "notes": "Regular shift"
                }
            }),
            &ctx,
        )
        .await
        .expect("add should succeed");

    assert_eq!(result.result["status"], "created");
    let record_id = result.result["record_id"].as_str().unwrap();
    assert!(!record_id.is_empty());

    // Query via tool
    let result = tool
        .execute(json!({"operation": "query"}), &ctx)
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

    let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

    // Missing required field
    let err = tool
        .execute(
            json!({"operation": "add", "data": { "notes": "no date" }}),
            &ctx,
        )
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

    let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

    // Add a record
    let result = tool
        .execute(
            json!({
                "operation": "add",
                "data": {
                    "date": "2026-02-22",
                    "start_time": "2026-02-22T08:00:00Z"
                }
            }),
            &ctx,
        )
        .await
        .unwrap();
    let record_id = result.result["record_id"].as_str().unwrap().to_string();

    // Update via tool
    let result = tool
        .execute(
            json!({
                "operation": "update",
                "record_id": record_id,
                "data": {
                    "status": "completed",
                    "end_time": "2026-02-22T17:00:00Z",
                    "hours": 9.0
                }
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

    let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

    // Add and delete
    let result = tool
        .execute(
            json!({
                "operation": "add",
                "data": {
                    "date": "2026-02-22",
                    "start_time": "2026-02-22T08:00:00Z"
                }
            }),
            &ctx,
        )
        .await
        .unwrap();
    let record_id = result.result["record_id"].as_str().unwrap().to_string();

    let result = tool
        .execute(
            json!({"operation": "delete", "record_id": record_id}),
            &ctx,
        )
        .await
        .expect("delete should succeed");

    assert_eq!(result.result["status"], "deleted");

    // Verify deleted
    assert!(
        db.get_record("andrew", uuid::Uuid::parse_str(&record_id).unwrap())
            .await
            .is_err()
    );
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

    let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

    // Add items
    tool.execute(
        json!({"operation": "add", "data": { "name": "milk", "category": "dairy", "on_list": true }}),
        &ctx,
    )
    .await
    .unwrap();
    tool.execute(
        json!({"operation": "add", "data": { "name": "bread", "category": "pantry", "on_list": true }}),
        &ctx,
    )
    .await
    .unwrap();
    tool.execute(
        json!({"operation": "add", "data": { "name": "eggs", "category": "dairy", "on_list": false }}),
        &ctx,
    )
    .await
    .unwrap();

    // Query with filter
    let result = tool
        .execute(
            json!({
                "operation": "query",
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

    let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

    // Add shifts with hours
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

    // Sum hours
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
        .expect("summary should succeed");

    // The aggregation result is the raw value (e.g., 24.5 for sum)
    let agg = &result.result["aggregation"];
    let total: f64 = agg
        .as_f64()
        .unwrap_or_else(|| agg.as_str().unwrap_or("0").parse().unwrap_or(0.0));
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

    let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

    // Add items
    for name in ["milk", "bread", "eggs", "waffles"] {
        tool.execute(
            json!({"operation": "add", "data": { "name": name }}),
            &ctx,
        )
        .await
        .unwrap();
    }

    // Count
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
        .expect("count should succeed");

    let agg = &result.result["aggregation"];
    let count: f64 = agg
        .as_f64()
        .unwrap_or_else(|| agg.as_str().unwrap_or("0").parse().unwrap_or(0.0));
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

    // Each user gets their own unified tool instance (scoped by owner_user_id).
    let andrew_tool = UnifiedCollectionTool::new(schema.clone(), Arc::clone(&db), None, "andrew");
    let grace_tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "grace");

    let andrew_ctx = test_ctx("andrew");
    let grace_ctx = test_ctx("grace");

    // Andrew adds items via his tool
    andrew_tool
        .execute(json!({"operation": "add", "data": { "name": "waffles" }}), &andrew_ctx)
        .await
        .unwrap();
    andrew_tool
        .execute(json!({"operation": "add", "data": { "name": "milk" }}), &andrew_ctx)
        .await
        .unwrap();

    // Grace adds items via her tool
    grace_tool
        .execute(json!({"operation": "add", "data": { "name": "yogurt" }}), &grace_ctx)
        .await
        .unwrap();

    // Andrew sees 2 items
    let result = andrew_tool
        .execute(json!({"operation": "query"}), &andrew_ctx)
        .await
        .unwrap();
    assert_eq!(result.result["count"], 2);

    // Grace sees 1 item
    let result = grace_tool
        .execute(json!({"operation": "query"}), &grace_ctx)
        .await
        .unwrap();
    assert_eq!(result.result["count"], 1);
}

#[tokio::test]
async fn tool_names_dont_collide_across_users() {
    let (db, _dir) = setup().await;
    let schema = grocery_schema();

    db.register_collection("andrew", &schema).await.unwrap();
    db.register_collection("grace", &schema).await.unwrap();

    let andrew_tools = generate_collection_tools(&schema, Arc::clone(&db), None, "andrew");
    let grace_tools = generate_collection_tools(&schema, db, None, "grace");

    // Tool names must include owner prefix
    let andrew_names: Vec<&str> = andrew_tools.iter().map(|t| t.name()).collect();
    let grace_names: Vec<&str> = grace_tools.iter().map(|t| t.name()).collect();

    // No overlap — different prefixes prevent registry collisions
    for name in &andrew_names {
        assert!(
            !grace_names.contains(name),
            "tool name {name} appears in both andrew and grace — registry collision"
        );
    }

    // Verify prefixes are correct (unified mode: 1 tool per collection)
    assert_eq!(andrew_names.len(), 1, "unified mode: 1 tool per user per collection");
    assert_eq!(grace_names.len(), 1);
    assert!(andrew_names[0].starts_with("andrew_"),
        "andrew tool should start with andrew_: {andrew_names:?}");
    assert!(grace_names[0].starts_with("grace_"),
        "grace tool should start with grace_: {grace_names:?}");
}

// ==================== Per-user tool filtering ====================

#[tokio::test]
async fn tool_definitions_for_user_filters_by_owner() {
    let (db, _dir) = setup().await;
    let nanny = nanny_schema();
    let grocery = grocery_schema();

    // Register schemas for both users so tools can execute (not tested here,
    // but schemas need to be valid for tool construction).
    db.register_collection("andrew", &nanny).await.expect("register nanny for andrew");
    db.register_collection("grace", &grocery).await.expect("register grocery for grace");

    // Generate collection tools for two different users
    let andrew_tools = generate_collection_tools(&nanny, Arc::clone(&db), None, "andrew");
    let grace_tools = generate_collection_tools(&grocery, Arc::clone(&db), None, "grace");

    // Put them all in one shared registry (simulates multi-tenant IronClaw)
    let registry = ToolRegistry::new();
    for tool in andrew_tools {
        registry.register(tool).await;
    }
    for tool in grace_tools {
        registry.register(tool).await;
    }

    // Also register a built-in tool (no owner) to verify it appears for everyone
    registry.register_builtin_tools();

    let total = registry.tool_definitions().await;
    let andrew_defs = registry.tool_definitions_for_user("andrew", &[]).await;
    let grace_defs = registry.tool_definitions_for_user("grace", &[]).await;

    // Both users should see the built-in tools
    let builtin_count = total.iter().filter(|d| {
        !d.name.starts_with("andrew_") && !d.name.starts_with("grace_")
    }).count();
    assert!(builtin_count > 0, "should have built-in tools");

    let andrew_builtin = andrew_defs.iter().filter(|d| {
        !d.name.starts_with("andrew_") && !d.name.starts_with("grace_")
    }).count();
    assert_eq!(andrew_builtin, builtin_count, "andrew should see all built-in tools");

    let grace_builtin = grace_defs.iter().filter(|d| {
        !d.name.starts_with("andrew_") && !d.name.starts_with("grace_")
    }).count();
    assert_eq!(grace_builtin, builtin_count, "grace should see all built-in tools");

    // Andrew should see his 1 collection tool but NOT grace's (unified mode)
    let andrew_collection: Vec<&str> = andrew_defs.iter()
        .filter(|d| d.name.starts_with("andrew_"))
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(andrew_collection.len(), 1, "andrew should have 1 unified collection tool");
    assert!(andrew_defs.iter().all(|d| !d.name.starts_with("grace_")),
        "andrew should NOT see any grace_ tools");

    // Grace should see her 1 collection tool but NOT andrew's
    let grace_collection: Vec<&str> = grace_defs.iter()
        .filter(|d| d.name.starts_with("grace_"))
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(grace_collection.len(), 1, "grace should have 1 unified collection tool");
    assert!(grace_defs.iter().all(|d| !d.name.starts_with("andrew_")),
        "grace should NOT see any andrew_ tools");

    // Total should include everything (unified mode: 1 tool per user per collection)
    assert_eq!(total.len(), builtin_count + 2,
        "total should include all built-ins + 1 andrew + 1 grace tool");
    assert_eq!(andrew_defs.len(), builtin_count + 1);
    assert_eq!(grace_defs.len(), builtin_count + 1);
}

// ==================== Security: source_scope stripping ====================

/// Negative security test: an LLM (or malicious caller) that passes
/// `source_scope` in the register-tool parameters must NOT be able to bind
/// the new collection to another user's scope.  `CollectionRegisterTool`
/// must strip the field before persisting.
#[tokio::test]
async fn register_tool_strips_source_scope_from_params() {
    use ironclaw::tools::builtin::collections::CollectionRegisterTool;

    let (db, _dir) = setup().await;
    let registry = Arc::new(ToolRegistry::new());
    let ctx = test_ctx("attacker");

    let register_tool = CollectionRegisterTool::new(Arc::clone(&db), Arc::clone(&registry));

    // Attacker injects source_scope pointing at a victim user
    let result = register_tool
        .execute(
            json!({
                "collection": "evil_collection",
                "description": "Trying to hijack victim_user scope",
                "source_scope": "victim_user",
                "fields": {
                    "name": { "type": "text", "required": true }
                }
            }),
            &ctx,
        )
        .await
        .expect("register should succeed (source_scope silently stripped)");

    assert_eq!(result.result["status"], "registered");

    // Verify the persisted schema does NOT carry source_scope
    let schemas = db
        .list_collections("attacker")
        .await
        .expect("list_collections should succeed");

    let schema = schemas
        .iter()
        .find(|s| s.collection == "evil_collection")
        .expect("evil_collection should exist under attacker");

    assert!(
        schema.source_scope.is_none(),
        "source_scope MUST be None after register — was {:?}",
        schema.source_scope
    );

    // Also verify the collection was NOT registered under the victim
    let victim_schemas = db
        .list_collections("victim_user")
        .await
        .expect("list_collections for victim should succeed");

    assert!(
        victim_schemas
            .iter()
            .all(|s| s.collection != "evil_collection"),
        "evil_collection must NOT appear under victim_user's collections"
    );
}

// ==================== Drop Tool ====================

#[tokio::test]
async fn drop_tool_removes_tools_from_registry() {
    use ironclaw::tools::builtin::collections::CollectionDropTool;

    let (db, _dir) = setup().await;
    let schema = grocery_schema();
    let ctx = test_ctx("andrew");

    // Register the collection in the database
    db.register_collection("andrew", &schema)
        .await
        .expect("register schema");

    // Create a registry and register collection tools
    let registry = Arc::new(ToolRegistry::new());
    let tools = generate_collection_tools(&schema, Arc::clone(&db), None, "andrew");
    for tool in &tools {
        registry.register(Arc::clone(tool)).await;
    }

    // Verify unified tool is registered
    assert!(
        registry.has("andrew_grocery_items").await,
        "unified tool should be registered"
    );

    // Create and execute the drop tool
    let drop_tool = CollectionDropTool::new(Arc::clone(&db), Arc::clone(&registry));
    let result = drop_tool
        .execute(json!({ "collection": "grocery_items" }), &ctx)
        .await
        .expect("drop should succeed");

    assert_eq!(result.result["status"], "dropped");

    // Verify unified tool is removed
    assert!(
        !registry.has("andrew_grocery_items").await,
        "unified tool should be removed"
    );

    // The tools_removed list should contain the unified tool
    let removed = result.result["tools_removed"].as_array().unwrap();
    assert_eq!(removed.len(), 1, "should have removed 1 unified tool");
}

// ==================== Startup Bootstrap ====================

/// Regression test: collections registered in a prior session must have their
/// per-collection tools restored at startup via `initialize_collection_tools_for_users`.
///
/// This test simulates the restart scenario:
/// 1. Register collections in the database (simulating a prior session)
/// 2. Create a fresh, empty ToolRegistry (simulating a restart)
/// 3. Call `initialize_collection_tools_for_users` (the startup bootstrap)
/// 4. Verify per-collection tools are present in the registry
///
/// Without the startup bootstrap, per-collection tools are lost on restart
/// because only management tools (collections_list, etc.) are registered as
/// built-ins — per-collection tools are dynamically generated.
#[tokio::test]
async fn startup_bootstrap_restores_collection_tools() {
    use ironclaw::tools::builtin::collections::initialize_collection_tools_for_users;

    let (db, _dir) = setup().await;

    // --- Simulate prior session: register collections in the database ---
    let nanny = nanny_schema();
    let grocery = grocery_schema();
    db.register_collection("andrew", &nanny)
        .await
        .expect("register nanny for andrew");
    db.register_collection("grace", &grocery)
        .await
        .expect("register grocery for grace");

    // --- Simulate restart: fresh registry with no collection tools ---
    let registry = Arc::new(ToolRegistry::new());
    registry.register_builtin_tools();

    // Verify no collection tools exist yet
    assert!(
        !registry.has("andrew_nanny_shifts").await,
        "andrew's nanny_shifts should NOT exist before bootstrap"
    );
    assert!(
        !registry.has("grace_grocery_items").await,
        "grace's grocery_items should NOT exist before bootstrap"
    );

    // --- Startup bootstrap ---
    let user_ids = vec!["andrew".to_string(), "grace".to_string()];
    initialize_collection_tools_for_users(
        &user_ids,
        &db,
        &registry,
        None, // skills_dir
        None, // skill_registry
        None, // collection_write_tx
        None, // workspace_resolver
    )
    .await;

    // --- Verify unified tools are restored ---
    // Andrew's nanny_shifts tool
    assert!(
        registry.has("andrew_nanny_shifts").await,
        "andrew_nanny_shifts should exist after bootstrap"
    );

    // Grace's grocery_items tool
    assert!(
        registry.has("grace_grocery_items").await,
        "grace_grocery_items should exist after bootstrap"
    );

    // Cross-user isolation: andrew should NOT see grace's tools in filtered view
    let andrew_defs = registry.tool_definitions_for_user("andrew", &[]).await;
    assert!(
        andrew_defs.iter().all(|d| !d.name.starts_with("grace_")),
        "andrew should NOT see grace's tools after bootstrap"
    );
    let grace_defs = registry.tool_definitions_for_user("grace", &[]).await;
    assert!(
        grace_defs.iter().all(|d| !d.name.starts_with("andrew_")),
        "grace should NOT see andrew's tools after bootstrap"
    );
}

/// Regression test: bootstrap with a user who has no collections should not
/// error — it should be a no-op.
#[tokio::test]
async fn startup_bootstrap_skips_users_without_collections() {
    use ironclaw::tools::builtin::collections::initialize_collection_tools_for_users;

    let (db, _dir) = setup().await;

    // Register one collection for andrew only
    let nanny = nanny_schema();
    db.register_collection("andrew", &nanny)
        .await
        .expect("register nanny for andrew");

    let registry = Arc::new(ToolRegistry::new());
    let _builtin_count_before = registry.tool_definitions().await.len();

    // Bootstrap for both andrew and a nonexistent user
    let user_ids = vec![
        "andrew".to_string(),
        "nonexistent_user".to_string(),
    ];
    initialize_collection_tools_for_users(
        &user_ids,
        &db,
        &registry,
        None,
        None,
        None,
        None,
    )
    .await;

    // Andrew's tool should exist
    assert!(
        registry.has("andrew_nanny_shifts").await,
        "andrew_nanny_shifts should exist"
    );

    // No tools should be created for the nonexistent user
    let all_defs = registry.tool_definitions().await;
    let nonexistent_tools: Vec<_> = all_defs
        .iter()
        .filter(|d| d.name.starts_with("nonexistent_user_"))
        .collect();
    assert!(
        nonexistent_tools.is_empty(),
        "no tools should be created for nonexistent user"
    );
}
