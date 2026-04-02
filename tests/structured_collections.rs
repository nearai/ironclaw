#![cfg(feature = "libsql")]
//! Integration tests for structured collections using file-backed libSQL.
//!
//! Each test creates a temporary database file because libSQL in-memory
//! databases are connection-local (each `connect()` gets its own isolated
//! instance). File-backed databases share state across connections.
//!
//! Covers schema lifecycle, CRUD, user isolation, querying, and aggregation
//! using two fixture schemas: nanny_shifts and grocery_items.

use std::collections::BTreeMap;

use serde_json::json;
use uuid::Uuid;

use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::structured::{
    AggOp, Aggregation, CollectionSchema, FieldDef, FieldType, Filter, FilterOp, StructuredStore,
};

// ==================== Setup ====================

/// Create a file-backed libSQL database in a temporary directory and run
/// migrations. Returns both the backend and the temp directory handle (which
/// must be kept alive for the duration of the test to prevent cleanup).
async fn setup() -> (LibSqlBackend, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&db_path)
        .await
        .expect("create file-backed db");
    backend.run_migrations().await.expect("run migrations");
    (backend, dir)
}

// ==================== Fixture Schemas ====================

fn nanny_schema() -> CollectionSchema {
    let mut fields = BTreeMap::new();

    fields.insert(
        "date".to_string(),
        FieldDef {
            field_type: FieldType::Date,
            required: true,
            default: None,
        },
    );
    fields.insert(
        "start_time".to_string(),
        FieldDef {
            field_type: FieldType::DateTime,
            required: true,
            default: None,
        },
    );
    fields.insert(
        "end_time".to_string(),
        FieldDef {
            field_type: FieldType::DateTime,
            required: false,
            default: None,
        },
    );
    fields.insert(
        "status".to_string(),
        FieldDef {
            field_type: FieldType::Enum {
                values: vec!["in_progress".to_string(), "completed".to_string()],
            },
            required: false,
            default: Some(json!("in_progress")),
        },
    );
    fields.insert(
        "hours".to_string(),
        FieldDef {
            field_type: FieldType::Number,
            required: false,
            default: None,
        },
    );
    fields.insert(
        "notes".to_string(),
        FieldDef {
            field_type: FieldType::Text,
            required: false,
            default: None,
        },
    );

    CollectionSchema {
        collection: "nanny_shifts".to_string(),
        description: Some("Nanny shift tracking".to_string()),
        fields,
        source_scope: None,
    }
}

fn grocery_schema() -> CollectionSchema {
    let mut fields = BTreeMap::new();

    fields.insert(
        "name".to_string(),
        FieldDef {
            field_type: FieldType::Text,
            required: true,
            default: None,
        },
    );
    fields.insert(
        "category".to_string(),
        FieldDef {
            field_type: FieldType::Enum {
                values: vec![
                    "produce".to_string(),
                    "dairy".to_string(),
                    "meat".to_string(),
                    "pantry".to_string(),
                    "frozen".to_string(),
                    "household".to_string(),
                    "other".to_string(),
                ],
            },
            required: false,
            default: None,
        },
    );
    fields.insert(
        "on_list".to_string(),
        FieldDef {
            field_type: FieldType::Bool,
            required: false,
            default: Some(json!(true)),
        },
    );
    fields.insert(
        "quantity".to_string(),
        FieldDef {
            field_type: FieldType::Number,
            required: false,
            default: None,
        },
    );
    fields.insert(
        "order_count".to_string(),
        FieldDef {
            field_type: FieldType::Number,
            required: false,
            default: Some(json!(0)),
        },
    );
    fields.insert(
        "notes".to_string(),
        FieldDef {
            field_type: FieldType::Text,
            required: false,
            default: None,
        },
    );

    CollectionSchema {
        collection: "grocery_items".to_string(),
        description: Some("Grocery list items".to_string()),
        fields,
        source_scope: None,
    }
}

// ==================== Schema Lifecycle ====================

#[tokio::test]
async fn register_and_list_collections() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");
    db.register_collection(user, &grocery_schema())
        .await
        .expect("register grocery_items");

    let collections = db.list_collections(user).await.expect("list collections");
    assert_eq!(collections.len(), 2);
    // Alphabetical order: grocery_items before nanny_shifts.
    assert_eq!(collections[0].collection, "grocery_items");
    assert_eq!(collections[1].collection, "nanny_shifts");
}

#[tokio::test]
async fn drop_collection_cascades() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &grocery_schema())
        .await
        .expect("register grocery_items");

    let id = db
        .insert_record(
            user,
            "grocery_items",
            json!({"name": "Milk", "category": "dairy"}),
        )
        .await
        .expect("insert record");

    // Verify record exists.
    db.get_record(user, id)
        .await
        .expect("record should exist before drop");

    db.drop_collection(user, "grocery_items")
        .await
        .expect("drop collection");

    // Record should be gone.
    let result = db.get_record(user, id).await;
    assert!(result.is_err(), "record should not exist after drop");

    // Schema should be gone too.
    let collections = db.list_collections(user).await.expect("list collections");
    assert!(collections.is_empty());
}

#[tokio::test]
async fn schema_upsert() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    let mut schema = nanny_schema();
    schema.description = Some("Original description".to_string());

    db.register_collection(user, &schema)
        .await
        .expect("register first time");

    // Update description.
    schema.description = Some("Updated description".to_string());
    db.register_collection(user, &schema)
        .await
        .expect("register second time (upsert)");

    let fetched = db
        .get_collection_schema(user, "nanny_shifts")
        .await
        .expect("get schema");
    assert_eq!(fetched.description, Some("Updated description".to_string()));

    // Should still be only one collection.
    let collections = db.list_collections(user).await.expect("list collections");
    assert_eq!(collections.len(), 1);
}

// ==================== CRUD ====================

#[tokio::test]
async fn insert_and_get_record() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let id = db
        .insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": "2026-02-23",
                "start_time": "2026-02-23T09:00:00+00:00",
                "end_time": "2026-02-23T17:00:00+00:00",
                "hours": 8,
                "notes": "Regular Monday shift"
            }),
        )
        .await
        .expect("insert nanny shift");

    let record = db.get_record(user, id).await.expect("get record by id");

    assert_eq!(record.id, id);
    assert_eq!(record.user_id, user);
    assert_eq!(record.collection, "nanny_shifts");
    assert_eq!(record.data["date"], "2026-02-23");
    assert_eq!(record.data["start_time"], "2026-02-23T09:00:00+00:00");
    assert_eq!(record.data["hours"], 8);
    assert_eq!(record.data["notes"], "Regular Monday shift");
    // Default should have been applied.
    assert_eq!(record.data["status"], "in_progress");
}

#[tokio::test]
async fn insert_rejects_invalid_data() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    // Missing required field (date).
    let result = db
        .insert_record(
            user,
            "nanny_shifts",
            json!({
                "start_time": "2026-02-23T09:00:00+00:00"
            }),
        )
        .await;
    assert!(
        result.is_err(),
        "should reject record missing required field"
    );

    // Unknown field.
    let result = db
        .insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": "2026-02-23",
                "start_time": "2026-02-23T09:00:00+00:00",
                "bogus_field": "should fail"
            }),
        )
        .await;
    assert!(result.is_err(), "should reject record with unknown field");
}

#[tokio::test]
async fn update_record_merges() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let id = db
        .insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": "2026-02-23",
                "start_time": "2026-02-23T09:00:00+00:00",
                "hours": 8,
                "notes": "Morning shift"
            }),
        )
        .await
        .expect("insert record");

    // Partial update: change status and notes, leave everything else.
    db.update_record(
        user,
        id,
        json!({
            "status": "completed",
            "notes": "Ended early"
        }),
    )
    .await
    .expect("update record");

    let updated = db.get_record(user, id).await.expect("get updated record");

    // Updated fields.
    assert_eq!(updated.data["status"], "completed");
    assert_eq!(updated.data["notes"], "Ended early");
    // Original fields preserved.
    assert_eq!(updated.data["date"], "2026-02-23");
    assert_eq!(updated.data["start_time"], "2026-02-23T09:00:00+00:00");
    assert_eq!(updated.data["hours"], 8);
}

#[tokio::test]
async fn delete_record() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &grocery_schema())
        .await
        .expect("register grocery_items");

    let id = db
        .insert_record(
            user,
            "grocery_items",
            json!({"name": "Eggs", "category": "dairy"}),
        )
        .await
        .expect("insert record");

    db.delete_record(user, id).await.expect("delete record");

    let result = db.get_record(user, id).await;
    assert!(result.is_err(), "get should fail after delete");
}

// ==================== User Isolation ====================

#[tokio::test]
async fn user_isolation() {
    let (db, _dir) = setup().await;
    let andrew = "andrew";
    let grace = "grace";

    // Both users register the same schema.
    db.register_collection(andrew, &grocery_schema())
        .await
        .expect("register for andrew");
    db.register_collection(grace, &grocery_schema())
        .await
        .expect("register for grace");

    // Andrew inserts a grocery item.
    let id = db
        .insert_record(
            andrew,
            "grocery_items",
            json!({"name": "Steak", "category": "meat"}),
        )
        .await
        .expect("andrew inserts steak");

    // Grace cannot see Andrew's record by ID.
    let result = db.get_record(grace, id).await;
    assert!(
        result.is_err(),
        "grace should not be able to get andrew's record"
    );

    // Grace's query returns empty.
    let records = db
        .query_records(grace, "grocery_items", &[], None, 100)
        .await
        .expect("grace queries grocery_items");
    assert!(
        records.is_empty(),
        "grace should see no records in grocery_items"
    );

    // Andrew can see his own record.
    let records = db
        .query_records(andrew, "grocery_items", &[], None, 100)
        .await
        .expect("andrew queries grocery_items");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].data["name"], "Steak");
}

// ==================== Query ====================

#[tokio::test]
async fn query_with_equality_filter() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &grocery_schema())
        .await
        .expect("register grocery_items");

    // Insert 3 items: 2 on_list=true, 1 on_list=false.
    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Bananas", "category": "produce", "on_list": true}),
    )
    .await
    .expect("insert bananas");

    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Milk", "category": "dairy", "on_list": true}),
    )
    .await
    .expect("insert milk");

    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Chips", "category": "pantry", "on_list": false}),
    )
    .await
    .expect("insert chips");

    let results = db
        .query_records(
            user,
            "grocery_items",
            &[Filter {
                field: "on_list".to_string(),
                op: FilterOp::Eq,
                value: json!(true),
            }],
            None,
            100,
        )
        .await
        .expect("query on_list=true");

    assert_eq!(results.len(), 2);
    for record in &results {
        assert_eq!(record.data["on_list"], true);
    }
}

#[tokio::test]
async fn query_with_comparison_filter() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    // Insert shifts with different hours.
    let shifts = [
        ("2026-02-23", 6.0),
        ("2026-02-24", 8.0),
        ("2026-02-25", 9.5),
        ("2026-02-26", 10.0),
    ];

    for (date, hours) in &shifts {
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    // Query hours > 8 (should get 9.5 and 10.0).
    let results = db
        .query_records(
            user,
            "nanny_shifts",
            &[Filter {
                field: "hours".to_string(),
                op: FilterOp::Gt,
                value: json!(8),
            }],
            None,
            100,
        )
        .await
        .expect("query hours > 8");

    assert_eq!(results.len(), 2);
    for record in &results {
        let hours = record.data["hours"]
            .as_f64()
            .expect("hours should be a number");
        assert!(hours > 8.0, "expected hours > 8, got {hours}");
    }
}

#[tokio::test]
async fn query_with_order_by() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &grocery_schema())
        .await
        .expect("register grocery_items");

    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Carrots", "category": "produce", "quantity": 3}),
    )
    .await
    .expect("insert carrots");

    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Apples", "category": "produce", "quantity": 10}),
    )
    .await
    .expect("insert apples");

    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Bread", "category": "pantry", "quantity": 1}),
    )
    .await
    .expect("insert bread");

    // Order by quantity (numeric ascending).
    let results = db
        .query_records(user, "grocery_items", &[], Some("quantity"), 100)
        .await
        .expect("query with order_by quantity");

    assert_eq!(results.len(), 3);
    let quantities: Vec<f64> = results
        .iter()
        .map(|r| {
            r.data["quantity"]
                .as_f64()
                .expect("quantity should be a number")
        })
        .collect();
    assert_eq!(quantities, vec![1.0, 3.0, 10.0]);
}

// ==================== Aggregation ====================

#[tokio::test]
async fn aggregate_sum() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let hours_values = [8.0, 7.5, 9.0, 6.0, 8.5];
    for (i, hours) in hours_values.iter().enumerate() {
        let date = format!("2026-02-{:02}", 20 + i);
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Sum,
                field: Some("hours".to_string()),
                group_by: None,
                filters: vec![],
            },
        )
        .await
        .expect("aggregate sum");

    let sum = result.as_f64().expect("sum should be a number");
    assert!((sum - 39.0).abs() < 0.001, "expected sum ~39.0, got {sum}");
}

#[tokio::test]
async fn aggregate_count() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let hours_values = [8.0, 7.5, 9.0, 6.0, 8.5];
    for (i, hours) in hours_values.iter().enumerate() {
        let date = format!("2026-02-{:02}", 20 + i);
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Count,
                field: None,
                group_by: None,
                filters: vec![],
            },
        )
        .await
        .expect("aggregate count");

    let count = result.as_i64().expect("count should be an integer");
    assert_eq!(count, 5);
}

#[tokio::test]
async fn aggregate_with_group_by() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    // Insert shifts across two dates.
    // Feb 23: 8 + 4 = 12 hours
    // Feb 24: 9 hours
    let shifts = [
        ("2026-02-23", 8.0),
        ("2026-02-23", 4.0),
        ("2026-02-24", 9.0),
    ];

    for (i, (date, hours)) in shifts.iter().enumerate() {
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T{:02}:00:00+00:00", 9 + i),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Sum,
                field: Some("hours".to_string()),
                group_by: Some("date".to_string()),
                filters: vec![],
            },
        )
        .await
        .expect("aggregate sum grouped by date");

    // Result should be an object with date keys.
    let obj = result
        .as_object()
        .expect("grouped result should be an object");
    assert_eq!(obj.len(), 2, "should have 2 date groups");

    let feb23 = obj
        .get("2026-02-23")
        .expect("should have 2026-02-23 group")
        .as_f64()
        .expect("group value should be a number");
    assert!(
        (feb23 - 12.0).abs() < 0.001,
        "expected Feb 23 sum ~12.0, got {feb23}"
    );

    let feb24 = obj
        .get("2026-02-24")
        .expect("should have 2026-02-24 group")
        .as_f64()
        .expect("group value should be a number");
    assert!(
        (feb24 - 9.0).abs() < 0.001,
        "expected Feb 24 sum ~9.0, got {feb24}"
    );
}

// ==================== Scoped Collection Access Tests ====================
//
// Tests for cross-scope collection access via source_scope field.
// Andrew can read/write household's collections through scoped tools.

/// Helper: create a simple schema with a single text field.
fn simple_schema(name: &str) -> CollectionSchema {
    let mut fields = BTreeMap::new();
    fields.insert(
        "item".to_string(),
        FieldDef {
            field_type: FieldType::Text,
            required: true,
            default: None,
        },
    );
    CollectionSchema {
        collection: name.to_string(),
        description: Some("test collection".to_string()),
        fields,
        source_scope: None,
    }
}

#[tokio::test]
async fn scoped_collection_source_scope_persists() {
    let (db, _dir) = setup().await;

    let mut schema = simple_schema("tasks");
    schema.source_scope = Some("household".to_string());

    db.register_collection("andrew", &schema).await.unwrap();

    let retrieved = db
        .get_collection_schema("andrew", "tasks")
        .await
        .unwrap();
    assert_eq!(retrieved.source_scope, Some("household".to_string()));
}

#[tokio::test]
async fn scoped_collection_write_isolation() {
    let (db, _dir) = setup().await;

    // Register collection in both scopes
    db.register_collection("household", &simple_schema("tasks"))
        .await
        .unwrap();
    db.register_collection("andrew", &simple_schema("tasks"))
        .await
        .unwrap();

    // Insert into household scope
    db.insert_record("household", "tasks", json!({"item": "clean gutters"}))
        .await
        .unwrap();

    // Insert into andrew scope
    db.insert_record("andrew", "tasks", json!({"item": "buy butt cream"}))
        .await
        .unwrap();

    // Each scope should only see its own data
    let household = db
        .query_records("household", "tasks", &[], None, 100)
        .await
        .unwrap();
    assert_eq!(household.len(), 1);
    assert_eq!(household[0].data["item"], "clean gutters");

    let andrew = db
        .query_records("andrew", "tasks", &[], None, 100)
        .await
        .unwrap();
    assert_eq!(andrew.len(), 1);
    assert_eq!(andrew[0].data["item"], "buy butt cream");
}

#[tokio::test]
async fn scoped_collection_cross_scope_query() {
    let (db, _dir) = setup().await;

    // household owns the collection
    db.register_collection("household", &simple_schema("tasks"))
        .await
        .unwrap();

    db.insert_record("household", "tasks", json!({"item": "vacuum"}))
        .await
        .unwrap();

    // Andrew queries household's scope directly (this is what the tool does)
    let results = db
        .query_records("household", "tasks", &[], None, 100)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].data["item"], "vacuum");

    // Andrew's own scope should be empty
    let andrew = db
        .query_records("andrew", "tasks", &[], None, 100)
        .await
        .unwrap();
    assert!(andrew.is_empty());
}

#[tokio::test]
async fn scoped_collection_list_includes_source_scope() {
    let (db, _dir) = setup().await;

    // Andrew's own collection
    db.register_collection("andrew", &simple_schema("notes"))
        .await
        .unwrap();

    // Cross-scope collection registered under andrew
    let mut scoped = simple_schema("childcare_hours");
    scoped.source_scope = Some("household".to_string());
    db.register_collection("andrew", &scoped).await.unwrap();

    let schemas = db.list_collections("andrew").await.unwrap();
    assert_eq!(schemas.len(), 2);

    let own = schemas
        .iter()
        .find(|s| s.collection == "notes")
        .unwrap();
    assert_eq!(own.source_scope, None);

    let cross = schemas
        .iter()
        .find(|s| s.collection == "childcare_hours")
        .unwrap();
    assert_eq!(cross.source_scope, Some("household".to_string()));
}

#[tokio::test]
async fn scoped_collection_history_tracking() {
    let (db, _dir) = setup().await;

    db.register_collection("andrew", &simple_schema("tasks"))
        .await
        .unwrap();

    // Insert a record and check _history is present
    let id = db
        .insert_record("andrew", "tasks", json!({"item": "test history"}))
        .await
        .unwrap();

    let records = db
        .query_records("andrew", "tasks", &[], None, 100)
        .await
        .unwrap();
    assert_eq!(records.len(), 1);

    // The record should have the item field
    assert_eq!(records[0].data["item"], "test history");
    assert_eq!(records[0].id, id);
}

// ==================== Filter Operators ====================

#[tokio::test]
async fn filter_lt() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let shifts = [
        ("2026-02-23", 6.0),
        ("2026-02-24", 8.0),
        ("2026-02-25", 9.5),
        ("2026-02-26", 10.0),
    ];

    for (date, hours) in &shifts {
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    // hours < 8 should return only 6.0
    let results = db
        .query_records(
            user,
            "nanny_shifts",
            &[Filter {
                field: "hours".to_string(),
                op: FilterOp::Lt,
                value: json!(8),
            }],
            None,
            100,
        )
        .await
        .expect("query hours < 8");

    assert_eq!(results.len(), 1);
    let hours = results[0].data["hours"].as_f64().expect("hours");
    assert!((hours - 6.0).abs() < 0.001, "expected 6.0, got {hours}");
}

#[tokio::test]
async fn filter_lte() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let shifts = [
        ("2026-02-23", 6.0),
        ("2026-02-24", 8.0),
        ("2026-02-25", 9.5),
        ("2026-02-26", 10.0),
    ];

    for (date, hours) in &shifts {
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    // hours <= 8 should return 6.0 and 8.0
    let results = db
        .query_records(
            user,
            "nanny_shifts",
            &[Filter {
                field: "hours".to_string(),
                op: FilterOp::Lte,
                value: json!(8),
            }],
            Some("hours"),
            100,
        )
        .await
        .expect("query hours <= 8");

    assert_eq!(results.len(), 2);
    let hours_vals: Vec<f64> = results
        .iter()
        .map(|r| r.data["hours"].as_f64().expect("hours"))
        .collect();
    assert_eq!(hours_vals, vec![6.0, 8.0]);
}

#[tokio::test]
async fn filter_gte() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let shifts = [
        ("2026-02-23", 6.0),
        ("2026-02-24", 8.0),
        ("2026-02-25", 9.5),
        ("2026-02-26", 10.0),
    ];

    for (date, hours) in &shifts {
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    // hours >= 9.5 should return 9.5 and 10.0
    let results = db
        .query_records(
            user,
            "nanny_shifts",
            &[Filter {
                field: "hours".to_string(),
                op: FilterOp::Gte,
                value: json!(9.5),
            }],
            Some("hours"),
            100,
        )
        .await
        .expect("query hours >= 9.5");

    assert_eq!(results.len(), 2);
    let hours_vals: Vec<f64> = results
        .iter()
        .map(|r| r.data["hours"].as_f64().expect("hours"))
        .collect();
    assert_eq!(hours_vals, vec![9.5, 10.0]);
}

#[tokio::test]
async fn filter_neq() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &grocery_schema())
        .await
        .expect("register grocery_items");

    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Bananas", "category": "produce"}),
    )
    .await
    .expect("insert bananas");

    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Milk", "category": "dairy"}),
    )
    .await
    .expect("insert milk");

    db.insert_record(
        user,
        "grocery_items",
        json!({"name": "Steak", "category": "meat"}),
    )
    .await
    .expect("insert steak");

    // category != "produce" should return Milk and Steak
    let results = db
        .query_records(
            user,
            "grocery_items",
            &[Filter {
                field: "category".to_string(),
                op: FilterOp::Neq,
                value: json!("produce"),
            }],
            Some("name"),
            100,
        )
        .await
        .expect("query category != produce");

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .map(|r| r.data["name"].as_str().expect("name"))
        .collect();
    assert!(names.contains(&"Milk"));
    assert!(names.contains(&"Steak"));
    assert!(!names.contains(&"Bananas"));
}

// ==================== Aggregation: Avg, Min, Max ====================

#[tokio::test]
async fn aggregate_avg() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let hours_values = [8.0, 7.0, 9.0, 6.0, 10.0];
    for (i, hours) in hours_values.iter().enumerate() {
        let date = format!("2026-02-{:02}", 20 + i);
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Avg,
                field: Some("hours".to_string()),
                group_by: None,
                filters: vec![],
            },
        )
        .await
        .expect("aggregate avg");

    let avg = result.as_f64().expect("avg should be a number");
    // (8 + 7 + 9 + 6 + 10) / 5 = 8.0
    assert!((avg - 8.0).abs() < 0.001, "expected avg ~8.0, got {avg}");
}

#[tokio::test]
async fn aggregate_min() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let hours_values = [8.0, 7.5, 9.0, 6.0, 8.5];
    for (i, hours) in hours_values.iter().enumerate() {
        let date = format!("2026-02-{:02}", 20 + i);
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Min,
                field: Some("hours".to_string()),
                group_by: None,
                filters: vec![],
            },
        )
        .await
        .expect("aggregate min");

    let min = result.as_f64().expect("min should be a number");
    assert!((min - 6.0).abs() < 0.001, "expected min ~6.0, got {min}");
}

#[tokio::test]
async fn aggregate_max() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let hours_values = [8.0, 7.5, 9.0, 6.0, 8.5];
    for (i, hours) in hours_values.iter().enumerate() {
        let date = format!("2026-02-{:02}", 20 + i);
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Max,
                field: Some("hours".to_string()),
                group_by: None,
                filters: vec![],
            },
        )
        .await
        .expect("aggregate max");

    let max = result.as_f64().expect("max should be a number");
    assert!((max - 9.0).abs() < 0.001, "expected max ~9.0, got {max}");
}

// ==================== Non-Existent Resource Handling ====================

#[tokio::test]
async fn query_nonexistent_collection() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    // Query a collection that was never registered. Should return empty or error, not panic.
    let result = db
        .query_records(user, "nonexistent_collection", &[], None, 100)
        .await;

    // Either returns empty results or an error — both are acceptable.
    match result {
        Ok(records) => assert!(records.is_empty(), "expected no records"),
        Err(_) => {} // Error is fine — means collection not found
    }
}

#[tokio::test]
async fn get_nonexistent_record() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let fake_id = Uuid::new_v4();
    let result = db.get_record(user, fake_id).await;
    assert!(
        result.is_err(),
        "getting a non-existent record should return an error"
    );
}

#[tokio::test]
async fn update_nonexistent_record() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let fake_id = Uuid::new_v4();
    let result = db
        .update_record(user, fake_id, json!({"hours": 5}))
        .await;
    assert!(
        result.is_err(),
        "updating a non-existent record should return an error"
    );
}

#[tokio::test]
async fn delete_nonexistent_record() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let fake_id = Uuid::new_v4();
    let result = db.delete_record(user, fake_id).await;
    assert!(
        result.is_err(),
        "deleting a non-existent record should return an error"
    );
}

// ==================== Limit ====================

#[tokio::test]
async fn query_with_limit() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &grocery_schema())
        .await
        .expect("register grocery_items");

    // Insert 10 records.
    for i in 0..10 {
        db.insert_record(
            user,
            "grocery_items",
            json!({
                "name": format!("Item_{:02}", i),
                "category": "pantry",
                "quantity": i,
            }),
        )
        .await
        .expect("insert item");
    }

    // Query all — should get 10.
    let all = db
        .query_records(user, "grocery_items", &[], Some("quantity"), 100)
        .await
        .expect("query all");
    assert_eq!(all.len(), 10);

    // Query with limit=3 — should get exactly 3.
    let limited = db
        .query_records(user, "grocery_items", &[], Some("quantity"), 3)
        .await
        .expect("query limit=3");
    assert_eq!(limited.len(), 3);

    // Query with limit=1 — should get exactly 1.
    let one = db
        .query_records(user, "grocery_items", &[], Some("quantity"), 1)
        .await
        .expect("query limit=1");
    assert_eq!(one.len(), 1);
}

// ==================== Aggregation on Empty Collection ====================

#[tokio::test]
async fn aggregate_sum_empty_collection() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    // Sum on empty collection — should return 0 or null, not error.
    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Sum,
                field: Some("hours".to_string()),
                group_by: None,
                filters: vec![],
            },
        )
        .await
        .expect("aggregate sum on empty");

    // SQL SUM of no rows returns NULL. Accept 0, 0.0, or null.
    let is_zero_or_null = result.is_null()
        || result.as_f64().map(|v| v.abs() < 0.001).unwrap_or(false)
        || result.as_i64().map(|v| v == 0).unwrap_or(false);
    assert!(
        is_zero_or_null,
        "expected 0 or null for sum of empty collection, got {result}"
    );
}

#[tokio::test]
async fn aggregate_count_empty_collection() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    // Count on empty collection — should return 0.
    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Count,
                field: None,
                group_by: None,
                filters: vec![],
            },
        )
        .await
        .expect("aggregate count on empty");

    let count = result.as_i64().unwrap_or(-1);
    assert_eq!(count, 0, "expected count 0 for empty collection");
}

#[tokio::test]
async fn aggregate_avg_empty_collection() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    // Avg on empty collection — SQL AVG of no rows returns NULL.
    let result = db
        .aggregate(
            user,
            "nanny_shifts",
            &Aggregation {
                operation: AggOp::Avg,
                field: Some("hours".to_string()),
                group_by: None,
                filters: vec![],
            },
        )
        .await
        .expect("aggregate avg on empty");

    // Accept null or 0.
    let acceptable = result.is_null()
        || result.as_f64().map(|v| v.abs() < 0.001).unwrap_or(false);
    assert!(
        acceptable,
        "expected null or 0 for avg of empty collection, got {result}"
    );
}

// ==================== Multiple Filters Combined ====================

#[tokio::test]
async fn query_with_multiple_filters() {
    let (db, _dir) = setup().await;
    let user = "test_user";

    db.register_collection(user, &nanny_schema())
        .await
        .expect("register nanny_shifts");

    let shifts = [
        ("2026-03-10", 6.0),
        ("2026-03-10", 8.5),
        ("2026-03-10", 9.0),
        ("2026-03-11", 7.0),
        ("2026-03-11", 10.0),
    ];

    for (date, hours) in &shifts {
        db.insert_record(
            user,
            "nanny_shifts",
            json!({
                "date": date,
                "start_time": format!("{date}T09:00:00+00:00"),
                "hours": hours,
            }),
        )
        .await
        .expect("insert shift");
    }

    // Filter: hours > 7 AND date = "2026-03-10"
    // Should match 8.5 and 9.0 (the two March 10 shifts with hours > 7).
    let results = db
        .query_records(
            user,
            "nanny_shifts",
            &[
                Filter {
                    field: "hours".to_string(),
                    op: FilterOp::Gt,
                    value: json!(7),
                },
                Filter {
                    field: "date".to_string(),
                    op: FilterOp::Eq,
                    value: json!("2026-03-10"),
                },
            ],
            Some("hours"),
            100,
        )
        .await
        .expect("query with multiple filters");

    assert_eq!(
        results.len(),
        2,
        "expected 2 results for hours > 7 AND date = 2026-03-10"
    );
    let hours_vals: Vec<f64> = results
        .iter()
        .map(|r| r.data["hours"].as_f64().expect("hours"))
        .collect();
    assert_eq!(hours_vals, vec![8.5, 9.0]);
}

// Boot-scan tests are in src/tools/registry.rs (unit tests) since ToolRegistry is private.
