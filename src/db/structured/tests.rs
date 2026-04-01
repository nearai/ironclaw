use std::collections::BTreeMap;

use crate::db::structured::{
    AlterOperation, Alteration, CollectionSchema, FieldDef, FieldType, ValidationError,
    append_history, init_history, is_system_field, validate_field_name,
};

// ==================== Fixture Schemas ====================

fn time_entry_schema() -> CollectionSchema {
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
            required: true,
            default: None,
        },
    );
    fields.insert(
        "status".to_string(),
        FieldDef {
            field_type: FieldType::Enum {
                values: vec![
                    "scheduled".to_string(),
                    "completed".to_string(),
                    "cancelled".to_string(),
                ],
            },
            required: false,
            default: Some(serde_json::json!("scheduled")),
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
        collection: "time_entries".to_string(),
        description: Some("Work time entry tracking".to_string()),
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
            default: Some(serde_json::json!(true)),
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
        "last_ordered".to_string(),
        FieldDef {
            field_type: FieldType::Date,
            required: false,
            default: None,
        },
    );
    fields.insert(
        "order_count".to_string(),
        FieldDef {
            field_type: FieldType::Number,
            required: false,
            default: Some(serde_json::json!(0)),
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

// ==================== Name Validation ====================

#[test]
fn valid_collection_names() {
    let long_name = "x".repeat(64);
    let names = ["time_entries", "grocery_items", "a", "A1_b2_c3", &long_name];
    for name in names {
        assert!(
            CollectionSchema::validate_name(name).is_ok(),
            "expected '{name}' to be valid"
        );
    }
}

#[test]
fn invalid_collection_names() {
    let cases = [
        ("", "empty"),
        ("_leading", "leading underscore"),
        ("has space", "space"),
        ("has-dash", "dash"),
        ("has.dot", "dot"),
        (&"x".repeat(65), "too long"),
    ];
    for (name, label) in &cases {
        assert!(
            CollectionSchema::validate_name(name).is_err(),
            "expected '{label}' name '{name}' to be invalid"
        );
    }
}

// ==================== Field Name Validation ====================

#[test]
fn valid_field_names() {
    let names = ["name", "start_time", "on_list", "a1_b2"];
    for name in names {
        assert!(
            validate_field_name(name).is_ok(),
            "expected field name '{name}' to be valid"
        );
    }
}

#[test]
fn field_name_rejects_sql_injection() {
    let cases = [
        ("'; DROP TABLE x; --", "SQL injection single quote"),
        ("a b", "space"),
        ("data->>'x'", "JSONB operator"),
        ("field-name", "dash"),
        ("field.name", "dot"),
    ];
    for (name, label) in &cases {
        assert!(
            validate_field_name(name).is_err(),
            "expected '{label}' field name to be rejected"
        );
    }
}

// ==================== Schema Round-Trip ====================

#[test]
fn schema_round_trip() {
    let schema = time_entry_schema();
    let json = serde_json::to_string(&schema).unwrap();
    let deserialized: CollectionSchema = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.collection, "time_entries");
    assert_eq!(deserialized.fields.len(), 5);
    assert!(deserialized.fields.contains_key("date"));
    assert!(deserialized.fields.contains_key("start_time"));
    assert!(deserialized.fields.contains_key("end_time"));
    assert!(deserialized.fields.contains_key("status"));
    assert!(deserialized.fields.contains_key("notes"));

    // Verify a specific field type round-trips correctly.
    let status = &deserialized.fields["status"];
    match &status.field_type {
        FieldType::Enum { values } => {
            assert_eq!(values.len(), 3);
            assert!(values.contains(&"scheduled".to_string()));
        }
        other => panic!("expected Enum, got {other:?}"),
    }
}

// ==================== Record Validation ====================

#[test]
fn valid_time_entry_record() {
    let schema = time_entry_schema();
    let data = serde_json::json!({
        "date": "2026-02-22",
        "start_time": "2026-02-22T09:00:00+00:00",
        "end_time": "2026-02-22T17:00:00+00:00",
        "notes": "Regular shift"
    });
    let result = schema.validate_record(&data).unwrap();

    // Status default should be applied.
    assert_eq!(result["status"], "scheduled");
    assert_eq!(result["date"], "2026-02-22");
    assert_eq!(result["notes"], "Regular shift");
}

#[test]
fn missing_required_field() {
    let schema = time_entry_schema();
    let data = serde_json::json!({
        "start_time": "2026-02-22T09:00:00+00:00",
        "end_time": "2026-02-22T17:00:00+00:00"
    });
    let err = schema.validate_record(&data).unwrap_err();
    match err {
        ValidationError::MissingRequired { field } => assert_eq!(field, "date"),
        other => panic!("expected MissingRequired, got {other}"),
    }
}

#[test]
fn unknown_field_rejected() {
    let schema = time_entry_schema();
    let data = serde_json::json!({
        "date": "2026-02-22",
        "start_time": "2026-02-22T09:00:00+00:00",
        "end_time": "2026-02-22T17:00:00+00:00",
        "bogus": "should fail"
    });
    let err = schema.validate_record(&data).unwrap_err();
    match err {
        ValidationError::UnknownField { field } => assert_eq!(field, "bogus"),
        other => panic!("expected UnknownField, got {other}"),
    }
}

#[test]
fn type_mismatch() {
    let schema = grocery_schema();
    let data = serde_json::json!({
        "name": "Milk",
        "quantity": "not a number"
    });
    let err = schema.validate_record(&data).unwrap_err();
    match err {
        ValidationError::TypeMismatch {
            field,
            expected,
            got,
        } => {
            assert_eq!(field, "quantity");
            assert_eq!(expected, "number");
            assert_eq!(got, "string");
        }
        other => panic!("expected TypeMismatch, got {other}"),
    }
}

#[test]
fn number_string_coercion() {
    let schema = grocery_schema();
    // LLMs sometimes send numbers as strings — should be accepted and coerced.
    let data = serde_json::json!({
        "name": "Eggs",
        "quantity": "12"
    });
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["quantity"], serde_json::json!(12));
    assert!(result["quantity"].is_number());

    // Float strings too.
    let data = serde_json::json!({
        "name": "Rice",
        "quantity": "2.5"
    });
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["quantity"], serde_json::json!(2.5));
}

#[test]
fn bool_string_coercion() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "active".to_string(),
        FieldDef {
            field_type: FieldType::Bool,
            required: true,
            default: None,
        },
    );
    let schema = CollectionSchema {
        collection: "test".to_string(),
        description: None,
        fields,
        source_scope: None,
    };
    let data = serde_json::json!({"active": "true"});
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["active"], serde_json::json!(true));

    let data = serde_json::json!({"active": "false"});
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["active"], serde_json::json!(false));

    // Non-boolean strings still fail.
    let data = serde_json::json!({"active": "yes"});
    assert!(schema.validate_record(&data).is_err());
}

#[test]
fn invalid_enum_value() {
    let schema = time_entry_schema();
    let data = serde_json::json!({
        "date": "2026-02-22",
        "start_time": "2026-02-22T09:00:00+00:00",
        "end_time": "2026-02-22T17:00:00+00:00",
        "status": "unknown_status"
    });
    let err = schema.validate_record(&data).unwrap_err();
    match err {
        ValidationError::InvalidEnumValue {
            field,
            value,
            allowed,
        } => {
            assert_eq!(field, "status");
            assert_eq!(value, "unknown_status");
            assert_eq!(allowed.len(), 3);
        }
        other => panic!("expected InvalidEnumValue, got {other}"),
    }
}

#[test]
fn invalid_date_format() {
    let schema = time_entry_schema();
    let data = serde_json::json!({
        "date": "22/02/2026",
        "start_time": "2026-02-22T09:00:00+00:00",
        "end_time": "2026-02-22T17:00:00+00:00"
    });
    let err = schema.validate_record(&data).unwrap_err();
    match err {
        ValidationError::InvalidDateFormat { field, .. } => assert_eq!(field, "date"),
        other => panic!("expected InvalidDateFormat, got {other}"),
    }
}

#[test]
fn invalid_datetime_format() {
    let schema = time_entry_schema();
    let data = serde_json::json!({
        "date": "2026-02-22",
        "start_time": "not-a-datetime",
        "end_time": "2026-02-22T17:00:00+00:00"
    });
    let err = schema.validate_record(&data).unwrap_err();
    match err {
        ValidationError::InvalidDateTimeFormat { field, .. } => {
            assert_eq!(field, "start_time");
        }
        other => panic!("expected InvalidDateTimeFormat, got {other}"),
    }
}

#[test]
fn bool_validation() {
    let schema = grocery_schema();

    // Valid bool.
    let data = serde_json::json!({
        "name": "Eggs",
        "on_list": false
    });
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["on_list"], false);

    // Invalid: string instead of bool.
    let bad = serde_json::json!({
        "name": "Eggs",
        "on_list": "yes"
    });
    let err = schema.validate_record(&bad).unwrap_err();
    match err {
        ValidationError::TypeMismatch { field, .. } => assert_eq!(field, "on_list"),
        other => panic!("expected TypeMismatch, got {other}"),
    }
}

#[test]
fn defaults_applied() {
    let schema = grocery_schema();
    let data = serde_json::json!({
        "name": "Bananas"
    });
    let result = schema.validate_record(&data).unwrap();

    // on_list defaults to true, order_count defaults to 0.
    assert_eq!(result["on_list"], true);
    assert_eq!(result["order_count"], 0);
    assert_eq!(result["name"], "Bananas");

    // category, quantity, last_ordered, notes are optional with no default -- absent.
    assert!(result.get("category").is_none());
    assert!(result.get("quantity").is_none());
    assert!(result.get("last_ordered").is_none());
    assert!(result.get("notes").is_none());
}

// ==================== Partial Update Validation ====================

#[test]
fn partial_update_valid() {
    let schema = time_entry_schema();
    let updates = serde_json::json!({
        "status": "completed",
        "notes": "Ended early"
    });
    let result = schema.validate_partial(&updates).unwrap();
    assert_eq!(result["status"], "completed");
    assert_eq!(result["notes"], "Ended early");
}

#[test]
fn partial_update_unknown_field() {
    let schema = time_entry_schema();
    let updates = serde_json::json!({
        "nonexistent": "value"
    });
    let err = schema.validate_partial(&updates).unwrap_err();
    match err {
        ValidationError::UnknownField { field } => assert_eq!(field, "nonexistent"),
        other => panic!("expected UnknownField, got {other}"),
    }
}

#[test]
fn partial_update_skips_required_check() {
    let schema = time_entry_schema();
    // Only updating notes -- should not complain about missing date/start_time/end_time.
    let updates = serde_json::json!({
        "notes": "Updated note"
    });
    let result = schema.validate_partial(&updates).unwrap();
    assert_eq!(result["notes"], "Updated note");
}

#[test]
fn partial_update_rejects_null_on_required_field() {
    let schema = time_entry_schema();
    // Setting a required field to null should fail.
    let updates = serde_json::json!({
        "date": null
    });
    let err = schema.validate_partial(&updates).unwrap_err();
    match err {
        ValidationError::MissingRequired { field } => assert_eq!(field, "date"),
        other => panic!("expected MissingRequired, got {other}"),
    }
}

#[test]
fn partial_update_allows_null_on_optional_field() {
    let schema = time_entry_schema();
    // Setting an optional field to null should pass.
    let updates = serde_json::json!({
        "notes": null
    });
    let result = schema.validate_partial(&updates).unwrap();
    assert_eq!(result["notes"], serde_json::Value::Null);
}

// ==================== System Field Support ====================

#[test]
fn is_system_field_detection() {
    assert!(is_system_field("_source"));
    assert!(is_system_field("_timestamp"));
    assert!(is_system_field("_ingested_at"));
    assert!(!is_system_field("source"));
    assert!(!is_system_field("name"));
    assert!(!is_system_field(""));
}

#[test]
fn system_fields_pass_through_validation() {
    let schema = time_entry_schema();
    let data = serde_json::json!({
        "date": "2026-02-22",
        "start_time": "2026-02-22T09:00:00+00:00",
        "end_time": "2026-02-22T17:00:00+00:00",
        "_source": "home_assistant",
        "_timestamp": "2026-02-22T09:00:00Z"
    });
    let result = schema.validate_record(&data).unwrap();

    // System fields should be preserved in output.
    assert_eq!(result["_source"], "home_assistant");
    assert_eq!(result["_timestamp"], "2026-02-22T09:00:00Z");
    // Regular fields still validated.
    assert_eq!(result["date"], "2026-02-22");
    // Default still applied.
    assert_eq!(result["status"], "scheduled");
}

#[test]
fn user_defined_underscore_field_rejected() {
    // validate_field_name rejects _ prefix for user-defined fields.
    assert!(validate_field_name("_source").is_err());
    assert!(validate_field_name("_timestamp").is_err());
}

// ==================== Schema Alteration ====================

#[test]
fn alter_add_field() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::AddField,
        field: "priority".to_string(),
        field_type: Some(FieldType::Enum {
            values: vec!["low".to_string(), "medium".to_string(), "high".to_string()],
        }),
        required: None,
        default: Some(serde_json::json!("medium")),
        value: None,
    };
    let result = schema.apply_alteration(&alt).unwrap();
    assert!(result.fields.contains_key("priority"));
    let def = &result.fields["priority"];
    assert!(!def.required);
    assert_eq!(def.default, Some(serde_json::json!("medium")));
    match &def.field_type {
        FieldType::Enum { values } => assert_eq!(values.len(), 3),
        other => panic!("expected Enum, got {other:?}"),
    }
}

#[test]
fn alter_add_field_already_exists() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::AddField,
        field: "name".to_string(),
        field_type: Some(FieldType::Text),
        required: None,
        default: None,
        value: None,
    };
    let err = schema.apply_alteration(&alt).unwrap_err();
    match err {
        ValidationError::InvalidName { name, .. } => assert_eq!(name, "name"),
        other => panic!("expected InvalidName, got {other}"),
    }
}

#[test]
fn alter_add_field_missing_type() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::AddField,
        field: "priority".to_string(),
        field_type: None,
        required: None,
        default: None,
        value: None,
    };
    assert!(schema.apply_alteration(&alt).is_err());
}

#[test]
fn alter_remove_field() {
    let schema = grocery_schema();
    assert!(schema.fields.contains_key("notes"));
    let alt = Alteration {
        operation: AlterOperation::RemoveField,
        field: "notes".to_string(),
        field_type: None,
        required: None,
        default: None,
        value: None,
    };
    let result = schema.apply_alteration(&alt).unwrap();
    assert!(!result.fields.contains_key("notes"));
}

#[test]
fn alter_remove_field_not_found() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::RemoveField,
        field: "nonexistent".to_string(),
        field_type: None,
        required: None,
        default: None,
        value: None,
    };
    let err = schema.apply_alteration(&alt).unwrap_err();
    match err {
        ValidationError::UnknownField { field } => assert_eq!(field, "nonexistent"),
        other => panic!("expected UnknownField, got {other}"),
    }
}

#[test]
fn alter_add_enum_value() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::AddEnumValue,
        field: "category".to_string(),
        field_type: None,
        required: None,
        default: None,
        value: Some("sweets".to_string()),
    };
    let result = schema.apply_alteration(&alt).unwrap();
    match &result.fields["category"].field_type {
        FieldType::Enum { values } => {
            assert!(values.contains(&"sweets".to_string()));
            assert_eq!(values.len(), 7); // 6 original + 1
        }
        other => panic!("expected Enum, got {other:?}"),
    }
}

#[test]
fn alter_add_enum_value_already_exists() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::AddEnumValue,
        field: "category".to_string(),
        field_type: None,
        required: None,
        default: None,
        value: Some("dairy".to_string()),
    };
    let err = schema.apply_alteration(&alt).unwrap_err();
    match err {
        ValidationError::InvalidEnumValue { field, value, .. } => {
            assert_eq!(field, "category");
            assert_eq!(value, "dairy");
        }
        other => panic!("expected InvalidEnumValue, got {other}"),
    }
}

#[test]
fn alter_add_enum_value_non_enum_field() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::AddEnumValue,
        field: "name".to_string(),
        field_type: None,
        required: None,
        default: None,
        value: Some("test".to_string()),
    };
    let err = schema.apply_alteration(&alt).unwrap_err();
    match err {
        ValidationError::TypeMismatch {
            field, expected, ..
        } => {
            assert_eq!(field, "name");
            assert_eq!(expected, "enum");
        }
        other => panic!("expected TypeMismatch, got {other}"),
    }
}

#[test]
fn alter_remove_enum_value() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::RemoveEnumValue,
        field: "category".to_string(),
        field_type: None,
        required: None,
        default: None,
        value: Some("other".to_string()),
    };
    let result = schema.apply_alteration(&alt).unwrap();
    match &result.fields["category"].field_type {
        FieldType::Enum { values } => {
            assert!(!values.contains(&"other".to_string()));
            assert_eq!(values.len(), 5); // 6 original - 1
        }
        other => panic!("expected Enum, got {other:?}"),
    }
}

#[test]
fn alter_remove_enum_value_not_found() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::RemoveEnumValue,
        field: "category".to_string(),
        field_type: None,
        required: None,
        default: None,
        value: Some("nonexistent".to_string()),
    };
    let err = schema.apply_alteration(&alt).unwrap_err();
    match err {
        ValidationError::InvalidEnumValue { field, value, .. } => {
            assert_eq!(field, "category");
            assert_eq!(value, "nonexistent");
        }
        other => panic!("expected InvalidEnumValue, got {other}"),
    }
}

// ==================== _lineage System Field ====================

#[test]
fn lineage_is_system_field() {
    assert!(is_system_field("_lineage"));
}

#[test]
fn lineage_passes_through_validation() {
    let schema = time_entry_schema();
    let data = serde_json::json!({
        "date": "2026-02-22",
        "start_time": "2026-02-22T09:00:00+00:00",
        "end_time": "2026-02-22T17:00:00+00:00",
        "_lineage": {
            "source": "conversation",
            "created_by": "user",
            "timestamp": "2026-02-22T09:00:00Z"
        }
    });
    let result = schema.validate_record(&data).unwrap();

    // _lineage should be preserved in output.
    let lineage = &result["_lineage"];
    assert_eq!(lineage["source"], "conversation");
    assert_eq!(lineage["created_by"], "user");
    assert_eq!(lineage["timestamp"], "2026-02-22T09:00:00Z");
    // Regular fields still validated.
    assert_eq!(result["date"], "2026-02-22");
    // Default still applied.
    assert_eq!(result["status"], "scheduled");
}

#[test]
fn lineage_with_full_provenance() {
    let schema = grocery_schema();
    let data = serde_json::json!({
        "name": "Milk",
        "_source": "webhook",
        "_timestamp": "2026-03-05T10:00:00Z",
        "_lineage": {
            "source": "webhook",
            "source_id": "evt-123",
            "created_by": "home_assistant",
            "context": "Grocery restock webhook",
            "timestamp": "2026-03-05T10:00:00Z"
        }
    });
    let result = schema.validate_record(&data).unwrap();

    assert_eq!(result["_source"], "webhook");
    assert_eq!(result["_lineage"]["source"], "webhook");
    assert_eq!(result["_lineage"]["source_id"], "evt-123");
    assert_eq!(result["_lineage"]["context"], "Grocery restock webhook");
}

// ==================== History Helper Tests ====================

#[test]
fn init_history_creates_single_insert_entry() {
    let mut data = serde_json::json!({
        "item": "milk",
        "quantity": 2,
        "_lineage": {"source": "conversation"}
    });
    init_history(&mut data, "conversation");

    let history = data["_history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["op"], "insert");
    assert_eq!(history[0]["source"], "conversation");
    assert_eq!(history[0]["fields"]["item"], "milk");
    assert_eq!(history[0]["fields"]["quantity"], 2);
    // System fields must not appear in the fields snapshot.
    assert!(history[0]["fields"].get("_lineage").is_none());
    assert!(history[0]["fields"].get("_history").is_none());
    assert!(history[0]["time"].as_str().is_some());
}

#[test]
fn append_history_adds_update_entry() {
    let mut data = serde_json::json!({
        "item": "milk",
        "_history": [
            {"op": "insert", "time": "2026-01-01T00:00:00Z", "source": "api", "fields": {"item": "milk"}}
        ]
    });
    let changed = serde_json::json!({"item": "oat milk"});
    append_history(&mut data, &changed, "rest_api");

    let history = data["_history"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1]["op"], "update");
    assert_eq!(history[1]["source"], "rest_api");
    assert_eq!(history[1]["fields"]["item"], "oat milk");
}

#[test]
fn append_history_creates_array_if_missing() {
    let mut data = serde_json::json!({"item": "old record"});
    let changed = serde_json::json!({"item": "updated"});
    append_history(&mut data, &changed, "api");

    let history = data["_history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["op"], "update");
}

#[test]
fn append_history_filters_system_fields_from_changed() {
    let mut data = serde_json::json!({
        "_history": [
            {"op": "insert", "time": "t", "source": "x", "fields": {}}
        ]
    });
    let changed = serde_json::json!({"quantity": 5, "_lineage": {"source": "api"}});
    append_history(&mut data, &changed, "api");

    let history = data["_history"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1]["fields"]["quantity"], 5);
    assert!(
        history[1]["fields"].get("_lineage").is_none(),
        "system fields should not appear in history fields"
    );
}

#[test]
fn validate_partial_passes_system_fields_through() {
    let schema = grocery_schema();
    let data = serde_json::json!({
        "name": "bread",
        "_history": [{"op": "insert"}]
    });
    let result = schema.validate_partial(&data).unwrap();
    assert_eq!(result["name"], "bread");
    assert_eq!(result["_history"][0]["op"], "insert");
}

// ==================== Edge Case: Empty Schema ====================

#[test]
fn schema_with_no_fields_validates_empty_record() {
    let schema = CollectionSchema {
        collection: "empty".to_string(),
        description: None,
        fields: BTreeMap::new(),
        source_scope: None,
    };
    let data = serde_json::json!({});
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result, serde_json::json!({}));
}

#[test]
fn schema_with_no_fields_rejects_unknown_field() {
    let schema = CollectionSchema {
        collection: "empty".to_string(),
        description: None,
        fields: BTreeMap::new(),
        source_scope: None,
    };
    let data = serde_json::json!({"foo": "bar"});
    assert!(matches!(
        schema.validate_record(&data),
        Err(ValidationError::UnknownField { .. })
    ));
}

#[test]
fn schema_with_no_fields_allows_system_fields() {
    let schema = CollectionSchema {
        collection: "empty".to_string(),
        description: None,
        fields: BTreeMap::new(),
        source_scope: None,
    };
    let data = serde_json::json!({"_source": "test", "_timestamp": "2025-01-01"});
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["_source"], "test");
    assert_eq!(result["_timestamp"], "2025-01-01");
}

// ==================== Edge Case: Coercion ====================

#[test]
fn number_coercion_integer_string() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "count".to_string(),
        FieldDef {
            field_type: FieldType::Number,
            required: true,
            default: None,
        },
    );
    let schema = CollectionSchema {
        collection: "nums".to_string(),
        description: None,
        fields,
        source_scope: None,
    };
    let data = serde_json::json!({"count": "8"});
    let result = schema.validate_record(&data).unwrap();
    // "8" should be coerced to 8
    assert_eq!(result["count"], serde_json::json!(8));
    assert!(result["count"].is_number());
}

#[test]
fn number_coercion_float_string() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "amount".to_string(),
        FieldDef {
            field_type: FieldType::Number,
            required: true,
            default: None,
        },
    );
    let schema = CollectionSchema {
        collection: "nums".to_string(),
        description: None,
        fields,
        source_scope: None,
    };
    let data = serde_json::json!({"amount": "7.5"});
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["amount"], serde_json::json!(7.5));
}

#[test]
fn bool_coercion_string_true() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "active".to_string(),
        FieldDef {
            field_type: FieldType::Bool,
            required: true,
            default: None,
        },
    );
    let schema = CollectionSchema {
        collection: "flags".to_string(),
        description: None,
        fields,
        source_scope: None,
    };
    let data = serde_json::json!({"active": "true"});
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["active"], serde_json::json!(true));
    assert!(result["active"].is_boolean());
}

#[test]
fn bool_coercion_string_false() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "active".to_string(),
        FieldDef {
            field_type: FieldType::Bool,
            required: true,
            default: None,
        },
    );
    let schema = CollectionSchema {
        collection: "flags".to_string(),
        description: None,
        fields,
        source_scope: None,
    };
    let data = serde_json::json!({"active": "false"});
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["active"], serde_json::json!(false));
}

// ==================== Edge Case: Validation Boundaries ====================

#[test]
fn validate_record_rejects_non_object() {
    let schema = grocery_schema();
    let data = serde_json::json!("not an object");
    assert!(matches!(
        schema.validate_record(&data),
        Err(ValidationError::TypeMismatch { field, .. }) if field == "(root)"
    ));
}

#[test]
fn validate_record_rejects_array() {
    let schema = grocery_schema();
    let data = serde_json::json!([{"name": "bread"}]);
    assert!(matches!(
        schema.validate_record(&data),
        Err(ValidationError::TypeMismatch { field, .. }) if field == "(root)"
    ));
}

#[test]
fn validate_record_rejects_null_for_required_field() {
    let schema = grocery_schema();
    let data = serde_json::json!({"name": null});
    assert!(matches!(
        schema.validate_record(&data),
        Err(ValidationError::MissingRequired { field }) if field == "name"
    ));
}

#[test]
fn validate_partial_rejects_non_object() {
    let schema = grocery_schema();
    let data = serde_json::json!(42);
    assert!(matches!(
        schema.validate_partial(&data),
        Err(ValidationError::TypeMismatch { field, .. }) if field == "(root)"
    ));
}

// ==================== Edge Case: Multiple System Fields ====================

#[test]
fn multiple_system_fields_all_pass_through() {
    let schema = grocery_schema();
    let data = serde_json::json!({
        "name": "milk",
        "_history": [{"op": "insert"}],
        "_lineage": {"source": "api"},
        "_source": "whatsapp",
        "_timestamp": "2025-01-01T00:00:00Z",
        "_custom_system": "anything"
    });
    let result = schema.validate_record(&data).unwrap();
    assert_eq!(result["_history"][0]["op"], "insert");
    assert_eq!(result["_lineage"]["source"], "api");
    assert_eq!(result["_source"], "whatsapp");
    assert_eq!(result["_timestamp"], "2025-01-01T00:00:00Z");
    assert_eq!(result["_custom_system"], "anything");
}

// ==================== Edge Case: Alteration Safety ====================

#[test]
fn alter_add_field_with_empty_enum_rejected() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::AddField,
        field: "status".to_string(),
        field_type: Some(FieldType::Enum {
            values: Vec::new(),
        }),
        required: None,
        default: None,
        value: None,
    };
    assert!(schema.apply_alteration(&alt).is_err());
}

#[test]
fn alter_add_field_validates_field_name() {
    let schema = grocery_schema();
    // Field name with spaces should be rejected
    let alt = Alteration {
        operation: AlterOperation::AddField,
        field: "bad field".to_string(),
        field_type: Some(FieldType::Text),
        required: None,
        default: None,
        value: None,
    };
    assert!(matches!(
        schema.apply_alteration(&alt),
        Err(ValidationError::InvalidName { .. })
    ));
}

#[test]
fn alter_add_field_rejects_system_field_name() {
    let schema = grocery_schema();
    let alt = Alteration {
        operation: AlterOperation::AddField,
        field: "_hidden".to_string(),
        field_type: Some(FieldType::Text),
        required: None,
        default: None,
        value: None,
    };
    assert!(matches!(
        schema.apply_alteration(&alt),
        Err(ValidationError::InvalidName { .. })
    ));
}

#[test]
fn alter_remove_enum_value_from_non_enum_field() {
    let schema = time_entry_schema();
    let alt = Alteration {
        operation: AlterOperation::RemoveEnumValue,
        field: "notes".to_string(), // Text field, not enum
        field_type: None,
        required: None,
        default: None,
        value: Some("anything".to_string()),
    };
    assert!(matches!(
        schema.apply_alteration(&alt),
        Err(ValidationError::TypeMismatch { .. })
    ));
}

// ==================== History Tracking Edge Cases ====================

#[test]
fn init_history_on_empty_record() {
    let mut data = serde_json::json!({});
    init_history(&mut data, "test");
    let history = data["_history"].as_array().expect("_history should be array");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["op"], "insert");
    assert_eq!(history[0]["source"], "test");
    // Fields should be an empty object (no user fields in empty record)
    assert_eq!(history[0]["fields"], serde_json::json!({}));
}

#[test]
fn init_history_filters_all_system_fields() {
    let mut data = serde_json::json!({
        "name": "test",
        "amount": 5,
        "_source": "api",
        "_timestamp": "2025-01-01",
        "_lineage": {"id": "abc"},
        "_internal": true
    });
    init_history(&mut data, "tool");
    let fields = &data["_history"][0]["fields"];
    assert_eq!(fields["name"], "test");
    assert_eq!(fields["amount"], 5);
    // None of the system fields should appear in the history snapshot
    assert!(fields.get("_source").is_none());
    assert!(fields.get("_timestamp").is_none());
    assert!(fields.get("_lineage").is_none());
    assert!(fields.get("_internal").is_none());
}

#[test]
fn append_history_multiple_updates() {
    let mut data = serde_json::json!({
        "name": "original",
        "_history": [{
            "op": "insert",
            "time": "2025-01-01T00:00:00Z",
            "source": "api",
            "fields": {"name": "original"}
        }]
    });
    let update1 = serde_json::json!({"name": "updated1"});
    append_history(&mut data, &update1, "tool");
    let update2 = serde_json::json!({"name": "updated2"});
    append_history(&mut data, &update2, "api");
    let history = data["_history"].as_array().unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0]["op"], "insert");
    assert_eq!(history[1]["op"], "update");
    assert_eq!(history[1]["fields"]["name"], "updated1");
    assert_eq!(history[2]["op"], "update");
    assert_eq!(history[2]["fields"]["name"], "updated2");
    assert_eq!(history[2]["source"], "api");
}

#[test]
fn append_history_on_non_object_data() {
    // append_history should handle non-object changed_fields gracefully
    let mut data = serde_json::json!({"_history": []});
    let changed = serde_json::json!("not an object");
    append_history(&mut data, &changed, "test");
    let history = data["_history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["fields"], serde_json::Value::Null);
}

// ==================== Name Validation Boundary Cases ====================

#[test]
fn collection_name_exactly_64_chars() {
    let name = "a".repeat(64);
    assert!(CollectionSchema::validate_name(&name).is_ok());
}

#[test]
fn collection_name_65_chars_rejected() {
    let name = "a".repeat(65);
    assert!(CollectionSchema::validate_name(&name).is_err());
}

#[test]
fn collection_name_single_char() {
    assert!(CollectionSchema::validate_name("x").is_ok());
}

#[test]
fn collection_name_all_underscores_after_first_char() {
    assert!(CollectionSchema::validate_name("a___").is_ok());
}

#[test]
fn collection_name_with_digits() {
    assert!(CollectionSchema::validate_name("item123").is_ok());
}

#[test]
fn collection_name_with_hyphen_rejected() {
    assert!(CollectionSchema::validate_name("my-collection").is_err());
}

#[test]
fn collection_name_with_space_rejected() {
    assert!(CollectionSchema::validate_name("my collection").is_err());
}

#[test]
fn collection_name_with_dot_rejected() {
    assert!(CollectionSchema::validate_name("my.collection").is_err());
}

// ==================== Default Validation ====================

#[test]
fn validate_defaults_rejects_type_mismatch() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "count".to_string(),
        FieldDef {
            field_type: FieldType::Number,
            required: false,
            default: Some(serde_json::json!("not a number")), // Wrong type
        },
    );
    let schema = CollectionSchema {
        collection: "bad_defaults".to_string(),
        description: None,
        fields,
        source_scope: None,
    };
    assert!(schema.validate_defaults().is_err());
}

#[test]
fn validate_defaults_accepts_valid_default() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "count".to_string(),
        FieldDef {
            field_type: FieldType::Number,
            required: false,
            default: Some(serde_json::json!(0)),
        },
    );
    let schema = CollectionSchema {
        collection: "good_defaults".to_string(),
        description: None,
        fields,
        source_scope: None,
    };
    assert!(schema.validate_defaults().is_ok());
}

// ==================== Time Validation ====================

#[test]
fn valid_time_hh_mm() {
    use crate::db::structured::validate_field_value;
    let ft = FieldType::Time;
    assert!(validate_field_value("t", &ft, &serde_json::json!("14:30")).is_ok());
}

#[test]
fn valid_time_hh_mm_ss() {
    use crate::db::structured::validate_field_value;
    let ft = FieldType::Time;
    assert!(validate_field_value("t", &ft, &serde_json::json!("14:30:59")).is_ok());
}

#[test]
fn invalid_time_format() {
    use crate::db::structured::validate_field_value;
    let ft = FieldType::Time;
    assert!(validate_field_value("t", &ft, &serde_json::json!("2pm")).is_err());
}

#[test]
fn time_rejects_non_string() {
    use crate::db::structured::validate_field_value;
    let ft = FieldType::Time;
    assert!(validate_field_value("t", &ft, &serde_json::json!(1430)).is_err());
}

// ==================== Source Scope ====================

#[test]
fn schema_with_source_scope_serializes() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "name".to_string(),
        FieldDef {
            field_type: FieldType::Text,
            required: true,
            default: None,
        },
    );
    let schema = CollectionSchema {
        collection: "shared_data".to_string(),
        description: Some("Cross-lens collection".to_string()),
        fields,
        source_scope: Some("household".to_string()),
    };
    let json = serde_json::to_value(&schema).unwrap();
    assert_eq!(json["source_scope"], "household");

    // Round-trip
    let deserialized: CollectionSchema = serde_json::from_value(json).unwrap();
    assert_eq!(deserialized.source_scope, Some("household".to_string()));
}

#[test]
fn schema_without_source_scope_omits_field() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "name".to_string(),
        FieldDef {
            field_type: FieldType::Text,
            required: true,
            default: None,
        },
    );
    let schema = CollectionSchema {
        collection: "private_data".to_string(),
        description: None,
        fields,
        source_scope: None,
    };
    let json = serde_json::to_value(&schema).unwrap();
    assert!(json.get("source_scope").is_none());
}

// ==================== json_to_text ====================

#[test]
fn json_to_text_conversions() {
    use crate::db::structured::json_to_text;
    assert_eq!(json_to_text(&serde_json::json!("hello")), "hello");
    assert_eq!(json_to_text(&serde_json::json!(42)), "42");
    assert_eq!(json_to_text(&serde_json::json!(3.15)), "3.15");
    assert_eq!(json_to_text(&serde_json::json!(true)), "true");
    assert_eq!(json_to_text(&serde_json::json!(false)), "false");
    assert_eq!(json_to_text(&serde_json::json!(null)), "");
}
