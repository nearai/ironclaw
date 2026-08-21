use super::*;

#[test]
fn provider_schema_requires_resolved_refs_without_restricting_canonical_shape() {
    assert!(provider_schema_is_resolved(
        &serde_json::json!({"type":"object"})
    ));
    assert!(provider_schema_is_resolved(
        &serde_json::json!({"type":"object","properties":{}})
    ));
    assert!(!provider_schema_is_resolved(&serde_json::json!({
        "$ref": "schemas/builtin/write-file.input.v1.json"
    })));
    assert!(provider_schema_is_resolved(&serde_json::json!({
        "$ref": "#/$defs/input",
        "$defs": {
            "input": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                }
            }
        }
    })));
    assert!(!provider_schema_is_resolved(&serde_json::json!({
        "type": "object",
        "properties": {
            "payload": {
                "$ref": "schemas/builtin/write-file.input.v1.json"
            }
        }
    })));
    assert!(provider_schema_is_resolved(&serde_json::json!({
        "oneOf": [
            {"type":"object","properties":{"action":{"const":"first"}}},
            {"type":"object","properties":{"action":{"const":"second"}}}
        ]
    })));
}

#[test]
fn provider_tool_name_is_bounded_and_uses_digest_entropy() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let mut existing = HashMap::new();
    existing.insert(
        ProviderToolName::new("demo__echo").expect("provider tool name"),
        CapabilityId::new("demo.other").expect("valid capability id"),
    );
    let name = provider_tool_name(&capability_id, &existing);

    assert!(name.as_str().len() <= PROVIDER_TOOL_NAME_MAX_BYTES);
    assert!(
        name.as_str()
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    );
    let suffix = name.as_str().rsplit("__").next().expect("digest suffix");
    assert_eq!(suffix.len(), PROVIDER_TOOL_NAME_DIGEST_BYTES);
    assert!(
        suffix
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
}

#[test]
fn provider_tool_name_normalizes_provider_unsafe_characters() {
    let capability_id = CapabilityId::new("demo.echo.v1").expect("valid capability id");
    let name = provider_tool_name(&capability_id, &HashMap::new());

    assert_eq!(name.as_str(), "demo__echo__v1");
    provider_validation::validate_provider_tool_name(name.as_str()).expect("provider-safe name");
}

#[test]
fn provider_tool_name_override_advertises_exactly_without_alias() {
    let capability_id = CapabilityId::new("builtin.read").expect("valid capability id");
    let override_name = ProviderToolName::new("read").expect("provider tool name");
    let advertised =
        resolve_provider_tool_name(&capability_id, Some(&override_name), &HashMap::new())
            .expect("override resolves");

    assert_eq!(advertised.as_str(), "read");
}

#[test]
fn provider_tool_name_without_override_derives_unchanged() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let advertised = resolve_provider_tool_name(&capability_id, None, &HashMap::new())
        .expect("derived name resolves");

    assert_eq!(advertised.as_str(), "demo__echo");
}

#[test]
fn provider_tool_name_override_collision_fails_loudly() {
    let capability_id = CapabilityId::new("builtin.read").expect("valid capability id");
    let mut existing = HashMap::new();
    existing.insert(
        ProviderToolName::new("read").expect("provider tool name"),
        CapabilityId::new("builtin.write").expect("valid capability id"),
    );
    let override_name = ProviderToolName::new("read").expect("provider tool name");
    let error = resolve_provider_tool_name(&capability_id, Some(&override_name), &existing)
        .expect_err("colliding override is rejected");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}
