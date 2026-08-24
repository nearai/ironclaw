use super::*;

#[test]
fn provider_argument_normalization_coerces_schema_declared_scalars() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer" },
            "enabled": { "type": "boolean" },
            "threshold": { "type": "number" },
            "message": { "type": "string" }
        }
    });
    let normalized = normalize_provider_arguments(
        &serde_json::json!({
            "limit": "10",
            "enabled": "true",
            "threshold": "1.5",
            "message": "10"
        }),
        &schema,
        "provider arguments",
    )
    .expect("normalized arguments");

    assert_eq!(
        normalized,
        serde_json::json!({
            "limit": 10,
            "enabled": true,
            "threshold": 1.5,
            "message": "10"
        })
    );
}

#[test]
fn provider_argument_normalization_coerces_stringified_containers() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "rows": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "index": { "type": "integer" },
                        "bold": { "type": "boolean" }
                    }
                }
            }
        }
    });
    let normalized = normalize_provider_arguments(
        &serde_json::json!({
            "rows": "[{\"index\":\"1\",\"bold\":\"false\"}]"
        }),
        &schema,
        "provider arguments",
    )
    .expect("normalized arguments");

    assert_eq!(
        normalized,
        serde_json::json!({
            "rows": [{ "index": 1, "bold": false }]
        })
    );
}

#[test]
fn provider_argument_normalization_rejects_invalid_schema_declared_integer() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer" }
        }
    });

    let error = normalize_provider_arguments(
        &serde_json::json!({ "limit": "ten" }),
        &schema,
        "provider arguments",
    )
    .expect_err("invalid integer should fail closed");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[test]
fn provider_argument_normalization_rejects_mismatched_stringified_object() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "options": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" }
                }
            }
        }
    });

    let error = normalize_provider_arguments(
        &serde_json::json!({ "options": "[{\"enabled\":\"true\"}]" }),
        &schema,
        "provider arguments",
    )
    .expect_err("stringified array should not satisfy object schema");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[test]
fn provider_argument_normalization_rejects_mismatched_stringified_array() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "rows": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "index": { "type": "integer" }
                    }
                }
            }
        }
    });

    let error = normalize_provider_arguments(
        &serde_json::json!({ "rows": "{\"index\":\"1\"}" }),
        &schema,
        "provider arguments",
    )
    .expect_err("stringified object should not satisfy array schema");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[test]
fn provider_argument_normalization_rejects_mismatched_stringified_array_without_items() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "rows": { "type": "array" }
        }
    });

    let error = normalize_provider_arguments(
        &serde_json::json!({ "rows": "{\"index\":\"1\"}" }),
        &schema,
        "provider arguments",
    )
    .expect_err("stringified object should not satisfy array schema without items");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

/// Regression: schemas like `headers` in `builtin.http` declare
/// `{ oneOf: [{type:object}, {type:array}] }` and have no top-level
/// `type`. Without `oneOf` handling, the normalizer's type-matched
/// branches never fire and a stringified array is forwarded raw to the
/// tool, which then rejects it with `InputEncode`.
#[test]
fn provider_argument_normalization_coerces_stringified_array_into_oneof_variant() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "headers": {
                "oneOf": [
                    { "type": "object", "additionalProperties": { "type": "string" } },
                    {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "value": { "type": "string" }
                            },
                            "required": ["name", "value"]
                        }
                    }
                ]
            }
        }
    });

    let normalized = normalize_provider_arguments(
        &serde_json::json!({
            "headers": "[{\"name\":\"User-Agent\",\"value\":\"IronClaw/1.0\"}]"
        }),
        &schema,
        "provider arguments",
    )
    .expect("oneOf array variant should accept stringified array");

    assert_eq!(
        normalized,
        serde_json::json!({
            "headers": [{ "name": "User-Agent", "value": "IronClaw/1.0" }]
        })
    );
}

#[test]
fn provider_argument_normalization_coerces_stringified_object_into_oneof_variant() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "headers": {
                "oneOf": [
                    { "type": "object", "additionalProperties": { "type": "string" } },
                    { "type": "array", "items": { "type": "object" } }
                ]
            }
        }
    });

    let normalized = normalize_provider_arguments(
        &serde_json::json!({
            "headers": "{\"User-Agent\":\"IronClaw/1.0\"}"
        }),
        &schema,
        "provider arguments",
    )
    .expect("oneOf object variant should accept stringified object");

    assert_eq!(
        normalized,
        serde_json::json!({
            "headers": { "User-Agent": "IronClaw/1.0" }
        })
    );
}

#[test]
fn provider_argument_normalization_passes_through_oneof_when_value_already_matches_variant() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "headers": {
                "oneOf": [
                    { "type": "object", "additionalProperties": { "type": "string" } },
                    { "type": "array", "items": { "type": "object" } }
                ]
            }
        }
    });

    let input = serde_json::json!({
        "headers": [{ "name": "X", "value": "y" }]
    });
    let normalized = normalize_provider_arguments(&input, &schema, "provider arguments")
        .expect("real array value should pass oneOf normalization unchanged");

    assert_eq!(normalized, input);
}

#[test]
fn provider_argument_normalization_anyof_behaves_like_oneof() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "payload": {
                "anyOf": [
                    { "type": "object" },
                    { "type": "array", "items": { "type": "string" } }
                ]
            }
        }
    });

    let normalized = normalize_provider_arguments(
        &serde_json::json!({ "payload": "[\"a\",\"b\"]" }),
        &schema,
        "provider arguments",
    )
    .expect("anyOf array variant should accept stringified array");

    assert_eq!(normalized, serde_json::json!({ "payload": ["a", "b"] }));
}

#[test]
fn provider_argument_preparation_validates_required_fields_before_dispatch() {
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "owner": { "type": "string" },
            "repo": { "type": "string" },
            "pr_number": { "type": "integer", "minimum": 1 }
        },
        "required": ["owner", "repo", "pr_number"]
    });

    let error = prepare_provider_arguments(
        &serde_json::json!({ "owner": "nearai", "repo": "ironclaw" }),
        &schema,
        "provider arguments",
    )
    .expect_err("missing required fields should fail before dispatch");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(error.safe_summary.contains("schema validation"));
    assert!(ironclaw_loop_contracts::LoopSafeSummary::new(error.safe_summary.clone()).is_ok());
}

#[test]
fn provider_argument_preparation_accepts_trigger_create_weekly_cron_schedule() {
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string" },
            "prompt": { "type": "string" },
            "schedule": {
                "oneOf": [
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "kind": { "const": "cron" },
                            "expression": { "type": "string" },
                            "timezone": { "type": "string" }
                        },
                        "required": ["kind", "expression", "timezone"]
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "kind": { "const": "once" },
                            "at": { "type": "string" },
                            "timezone": { "type": "string" }
                        },
                        "required": ["kind", "at", "timezone"]
                    }
                ]
            }
        },
        "required": ["name", "prompt", "schedule"]
    });

    let input = serde_json::json!({
        "name": "Tuesday reminder",
        "prompt": "Send the Tuesday reminder",
        "schedule": {
            "kind": "cron",
            "expression": "0 14 * * 2",
            "timezone": "America/Los_Angeles"
        }
    });

    let normalized = prepare_provider_arguments(&input, &schema, "provider arguments")
        .expect("trigger_create weekly cron arguments should pass provider validation");

    assert_eq!(normalized, input);

    let once_input = serde_json::json!({
        "name": "Dog walking reminder",
        "prompt": "Walk the dog",
        "schedule": {
            "kind": "once",
            "at": "2026-06-23T14:00:00",
            "timezone": "America/Los_Angeles"
        }
    });

    let normalized = prepare_provider_arguments(&once_input, &schema, "provider arguments")
        .expect("trigger_create once arguments should pass provider validation");

    assert_eq!(normalized, once_input);

    let stringified_schedule_input = serde_json::json!({
        "name": "Walk dog - Wednesdays",
        "prompt": "Reminder: It's time to walk your dog!",
        "schedule": "{\"kind\":\"cron\",\"expression\":\"0 15 * * 3\",\"timezone\":\"America/Los_Angeles\"}"
    });

    let normalized =
        prepare_provider_arguments(&stringified_schedule_input, &schema, "provider arguments")
            .expect("stringified trigger_create schedule should be decoded before validation");

    assert_eq!(
        normalized,
        serde_json::json!({
            "name": "Walk dog - Wednesdays",
            "prompt": "Reminder: It's time to walk your dog!",
            "schedule": {
                "kind": "cron",
                "expression": "0 15 * * 3",
                "timezone": "America/Los_Angeles"
            }
        })
    );
}

#[test]
fn provider_argument_preparation_rejects_unresolved_ref_schema() {
    let schema = serde_json::json!({
        "$ref": "schemas/demo/echo.input.v1.json"
    });

    let error = prepare_provider_arguments(
        &serde_json::json!({ "message": "hello" }),
        &schema,
        "provider arguments",
    )
    .expect_err("unresolved ref schemas must fail closed");

    assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
}

#[test]
fn provider_argument_preparation_rejects_nested_unresolved_ref_schema() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "payload": {
                "type": "object",
                "properties": {
                    "tool_input": {
                        "$ref": "schemas/demo/echo.input.v1.json"
                    }
                }
            }
        }
    });

    let error = prepare_provider_arguments(
        &serde_json::json!({
            "payload": {
                "tool_input": {
                    "message": "hello"
                }
            }
        }),
        &schema,
        "provider arguments",
    )
    .expect_err("nested unresolved refs must fail closed");

    assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
}

#[test]
fn provider_argument_preparation_allows_internal_ref_schema() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "payload": {
                "$ref": "#/$defs/payload"
            }
        },
        "$defs": {
            "payload": {
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"],
                "additionalProperties": false
            }
        }
    });

    let normalized = prepare_provider_arguments(
        &serde_json::json!({
            "payload": {
                "message": "hello"
            }
        }),
        &schema,
        "provider arguments",
    )
    .expect("internal refs should be allowed");

    assert_eq!(
        normalized,
        serde_json::json!({
            "payload": {
                "message": "hello"
            }
        })
    );
}

#[test]
fn provider_argument_preparation_rejects_excessive_schema_ref_scan_depth() {
    fn wrap_unknown_keyword(inner_schema: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "x-next": inner_schema
        })
    }

    let mut deep_annotation = serde_json::json!({ "type": "null" });
    for _ in 0..=provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH {
        deep_annotation = wrap_unknown_keyword(deep_annotation);
    }
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "message": { "type": "string" }
        },
        "required": ["message"],
        "x-adversarial-depth": deep_annotation
    });

    let error = prepare_provider_arguments(
        &serde_json::json!({ "message": "hello" }),
        &schema,
        "provider arguments",
    )
    .expect_err("excessively deep schema ref scans should fail closed");

    assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
}

#[test]
fn provider_argument_depth_limit_allows_exact_boundary() {
    fn wrap_object_property(name: String, inner_schema: serde_json::Value) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        properties.insert(name, inner_schema);
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), serde_json::json!("object"));
        schema.insert(
            "properties".to_string(),
            serde_json::Value::Object(properties),
        );
        serde_json::Value::Object(schema)
    }

    fn wrap_object_value(name: String, inner_value: serde_json::Value) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert(name, inner_value);
        serde_json::Value::Object(object)
    }

    fn wrap_unknown_keyword(inner_schema: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "x-next": inner_schema
        })
    }

    let mut schema = serde_json::json!({ "type": "integer" });
    let mut value = serde_json::json!("1");
    for depth in (0..provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH).rev() {
        let property = format!("level_{depth}");
        schema = wrap_object_property(property.clone(), schema);
        value = wrap_object_value(property, value);
    }

    let normalized = normalize_provider_arguments(&value, &schema, "provider arguments")
        .expect("exact normalization depth boundary should pass");

    assert_eq!(normalized, {
        let mut expected = serde_json::json!(1);
        for depth in (0..provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH).rev() {
            expected = wrap_object_value(format!("level_{depth}"), expected);
        }
        expected
    });

    let mut deep_annotation = serde_json::json!({ "type": "null" });
    for _ in 2..provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH {
        deep_annotation = wrap_unknown_keyword(deep_annotation);
    }
    let ref_scan_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "message": { "type": "string" }
        },
        "required": ["message"],
        "x-depth-boundary": deep_annotation
    });

    prepare_provider_arguments(
        &serde_json::json!({ "message": "hello" }),
        &ref_scan_schema,
        "provider arguments",
    )
    .expect("exact schema ref-scan depth boundary should pass");
}

#[test]
fn provider_argument_normalization_rejects_excessive_schema_depth() {
    fn wrap_object_property(name: String, inner_schema: serde_json::Value) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        properties.insert(name, inner_schema);
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), serde_json::json!("object"));
        schema.insert(
            "properties".to_string(),
            serde_json::Value::Object(properties),
        );
        serde_json::Value::Object(schema)
    }

    fn wrap_object_value(name: String, inner_value: serde_json::Value) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert(name, inner_value);
        serde_json::Value::Object(object)
    }

    let mut schema = serde_json::json!({ "type": "integer" });
    let mut value = serde_json::json!("1");
    for depth in (0..=provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH).rev() {
        let property = format!("level_{depth}");
        schema = wrap_object_property(property.clone(), schema);
        value = wrap_object_value(property, value);
    }

    let error = normalize_provider_arguments(&value, &schema, "provider arguments")
        .expect_err("excessively deep schema normalization should fail closed");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[test]
fn provider_argument_normalization_rejects_excessive_array_items_schema_depth() {
    fn wrap_array_schema(inner_schema: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "array",
            "items": inner_schema
        })
    }

    fn wrap_array_value(inner_value: serde_json::Value) -> serde_json::Value {
        serde_json::Value::Array(vec![inner_value])
    }

    let mut schema = serde_json::json!({ "type": "integer" });
    let mut value = serde_json::json!("1");
    for _ in 0..=provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH {
        schema = wrap_array_schema(schema);
        value = wrap_array_value(value);
    }

    let error = normalize_provider_arguments(&value, &schema, "provider arguments")
        .expect_err("excessively deep array item normalization should fail closed");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[test]
fn provider_argument_preparation_rejects_unknown_fields_before_dispatch() {
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "owner": { "type": "string" },
            "repo": { "type": "string" },
            "pr_number": { "type": "integer" }
        },
        "required": ["owner", "repo", "pr_number"]
    });

    let error = prepare_provider_arguments(
        &serde_json::json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "pr_number": 4286,
            "number": 4286
        }),
        &schema,
        "provider arguments",
    )
    .expect_err("additional properties should fail before dispatch");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(error.safe_summary.contains("schema validation"));
    assert!(ironclaw_loop_contracts::LoopSafeSummary::new(error.safe_summary.clone()).is_ok());
}

#[test]
fn provider_argument_preparation_validates_composed_object_schema_after_normalization() {
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": { "type": "string" },
            "page": { "type": "integer", "minimum": 1 },
            "owner": { "type": "string" },
            "repo": { "type": "string" }
        },
        "allOf": [
            {
                "if": { "required": ["owner"] },
                "then": { "required": ["repo"] }
            }
        ],
        "anyOf": [
            { "required": ["query"] },
            { "required": ["owner", "repo"] }
        ]
    });

    let normalized = prepare_provider_arguments(
        &serde_json::json!({ "query": "repo:nearai/ironclaw", "page": "2" }),
        &schema,
        "provider arguments",
    )
    .expect("top-level anyOf object schema should still normalize properties");
    assert_eq!(
        normalized,
        serde_json::json!({ "query": "repo:nearai/ironclaw", "page": 2 })
    );

    let error = prepare_provider_arguments(
        &serde_json::json!({ "owner": "nearai" }),
        &schema,
        "provider arguments",
    )
    .expect_err("composed schema constraints should fail before dispatch");
    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[test]
fn provider_argument_schema_failure_sanitizes_sensitive_path_markers() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "secret_api_key": { "type": "integer" }
        }
    });

    let error = prepare_provider_arguments(
        &serde_json::json!({ "secret_api_key": "not an integer" }),
        &schema,
        "provider arguments",
    )
    .expect_err("schema failure should remain a model-visible invocation error");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(!error.safe_summary.contains("secret"));
    assert!(!error.safe_summary.contains("api_key"));
}

/// Regression for Gemini review comment: a plain string that starts with
/// `{` or `[` but is not valid JSON must not cause an `InvalidInvocation`
/// error when a `string` variant is available. The coercion attempt should
/// fail gracefully and fall through to the string branch.
#[test]
fn provider_argument_normalization_oneof_string_variant_accepts_non_json_string() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "oneOf": [
                    { "type": "object" },
                    { "type": "string" }
                ]
            }
        }
    });

    // Looks like JSON but is malformed — must not error; string variant matches.
    let normalized = normalize_provider_arguments(
        &serde_json::json!({ "query": "{not valid json" }),
        &schema,
        "provider arguments",
    )
    .expect("malformed JSON-like string should fall through to the string variant");

    assert_eq!(
        normalized,
        serde_json::json!({ "query": "{not valid json" })
    );
}

/// Regression for Gemini review comment: JSON Schema treats every integer
/// as a valid number, so an integer-shaped value must match a `number`
/// variant in a `oneOf`/`anyOf` schema.
#[test]
fn provider_argument_normalization_oneof_integer_matches_number_variant() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "value": {
                "oneOf": [
                    { "type": "string" },
                    { "type": "number" }
                ]
            }
        }
    });

    let normalized = normalize_provider_arguments(
        &serde_json::json!({ "value": 42 }),
        &schema,
        "provider arguments",
    )
    .expect("integer value should match the number variant");

    assert_eq!(normalized, serde_json::json!({ "value": 42 }));
}
