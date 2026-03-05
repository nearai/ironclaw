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
