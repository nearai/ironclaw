use std::collections::BTreeMap;

use crate::db::structured::{
    CollectionSchema, FieldDef, FieldType, ValidationError, is_system_field, validate_field_name,
};

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
    }
}

// ==================== Name Validation ====================

#[test]
fn valid_collection_names() {
    let long_name = "x".repeat(64);
    let names = ["nanny_shifts", "grocery_items", "a", "A1_b2_c3", &long_name];
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
    let schema = nanny_schema();
    let json = serde_json::to_string(&schema).unwrap();
    let deserialized: CollectionSchema = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.collection, "nanny_shifts");
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
fn valid_nanny_record() {
    let schema = nanny_schema();
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
    let schema = nanny_schema();
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
    let schema = nanny_schema();
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
fn invalid_enum_value() {
    let schema = nanny_schema();
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
    let schema = nanny_schema();
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
    let schema = nanny_schema();
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
    let schema = nanny_schema();
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
    let schema = nanny_schema();
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
    let schema = nanny_schema();
    // Only updating notes -- should not complain about missing date/start_time/end_time.
    let updates = serde_json::json!({
        "notes": "Updated note"
    });
    let result = schema.validate_partial(&updates).unwrap();
    assert_eq!(result["notes"], "Updated note");
}

#[test]
fn partial_update_rejects_null_on_required_field() {
    let schema = nanny_schema();
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
    let schema = nanny_schema();
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
    let schema = nanny_schema();
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
