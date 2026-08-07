use super::*;

#[test]
fn advisory_model_route_carries_model_and_marks_itself_advisory() {
    let route = LoopModelRouteSnapshot::advisory("gpt-4o").expect("valid model");
    assert_eq!(route.model_id(), "gpt-4o");
    assert!(route.is_advisory());
    assert!(route.validate().is_ok());
}

#[test]
fn operator_resolved_route_is_not_advisory() {
    let route = LoopModelRouteSnapshot::new("openai", "gpt-4o", "config:v1", "auth:v1");
    assert!(!route.is_advisory());
}

#[test]
fn wire_shape_is_the_flat_four_component_object_and_round_trips() {
    // The enum refactor must not change the persisted shape: historical
    // stored routes (flat objects, advisory = the "requested" sentinel in
    // three components) must deserialize to the right variant AND serialize
    // back to the identical flat object, so pre-existing run records survive.
    let advisory_json = r#"{"provider_id":"requested","model_id":"gpt-4o","config_version":"requested","auth_version":"requested"}"#;
    let advisory: LoopModelRouteSnapshot =
        serde_json::from_str(advisory_json).expect("advisory route deserializes");
    assert_eq!(
        advisory,
        LoopModelRouteSnapshot::Advisory {
            model_id: "gpt-4o".to_string()
        }
    );
    assert!(advisory.is_advisory());
    assert_eq!(
        serde_json::to_string(&advisory).expect("serialize"),
        advisory_json
    );

    let resolved_json = r#"{"provider_id":"anthropic","model_id":"claude","config_version":"cfg:v1","auth_version":"auth:v1"}"#;
    let resolved: LoopModelRouteSnapshot =
        serde_json::from_str(resolved_json).expect("resolved route deserializes");
    assert_eq!(
        resolved,
        LoopModelRouteSnapshot::Resolved {
            provider_id: "anthropic".to_string(),
            model_id: "claude".to_string(),
            config_version: "cfg:v1".to_string(),
            auth_version: "auth:v1".to_string(),
        }
    );
    assert!(!resolved.is_advisory());
    assert_eq!(
        serde_json::to_string(&resolved).expect("serialize"),
        resolved_json
    );
}

#[test]
fn deserialize_validates_route_components() {
    // A well-formed operator route round-trips.
    let valid = LoopModelRouteSnapshot::new("openai", "gpt-4o", "config:v1", "auth:v1");
    let json = serde_json::to_string(&valid).expect("serialize");
    let restored: LoopModelRouteSnapshot = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored, valid);

    // Deserialization must not bypass validation: a secret-like component
    // that `new` would happily construct must be rejected on the wire so a
    // tampered/legacy snapshot cannot rehydrate into an unvalidated route.
    let secret_like = serde_json::json!({
        "provider_id": "sk-secret-provider",
        "model_id": "gpt-4",
        "config_version": "config:v1",
        "auth_version": "auth:v1",
    })
    .to_string();
    serde_json::from_str::<LoopModelRouteSnapshot>(&secret_like)
        .expect_err("secret-like provider_id must be rejected on deserialize");

    let forbidden_marker = serde_json::json!({
        "provider_id": "openrouter",
        "model_id": "gpt-4",
        "config_version": "config:api_key",
        "auth_version": "auth:v1",
    })
    .to_string();
    serde_json::from_str::<LoopModelRouteSnapshot>(&forbidden_marker)
        .expect_err("forbidden marker in config_version must be rejected on deserialize");
}

#[test]
fn advisory_model_route_trims_and_rejects_empty_or_invalid_models() {
    assert_eq!(LoopModelRouteSnapshot::advisory("   "), None);
    assert_eq!(LoopModelRouteSnapshot::advisory(""), None);
    // A model id with a space is not a valid route component → falls back.
    assert_eq!(LoopModelRouteSnapshot::advisory("gpt 4o"), None);
    // Surrounding whitespace is trimmed before validation.
    assert_eq!(
        LoopModelRouteSnapshot::advisory("  claude-opus-4-6  ")
            .map(|route| route.model_id().to_string()),
        Some("claude-opus-4-6".to_string())
    );
}
