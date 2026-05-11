use ironclaw_turns::run_profile::{
    InMemoryModelRouteResolver, ModelId, ModelRoute, ModelRouteResolutionError, ModelRouteResolver,
    ModelSelectionPolicy, ModelSlot, ProviderId, ResolvedModelRoute,
};

fn route(provider: &str, model: &str) -> ModelRoute {
    ModelRoute {
        provider_id: ProviderId::new(provider).unwrap(),
        model_id: ModelId::new(model).unwrap(),
    }
}

#[test]
fn developer_any_configured_accepts_any_configured_route() {
    let resolver = InMemoryModelRouteResolver;
    let requested = route("openai", "gpt-4o");
    let result = resolver
        .resolve_model_route(
            &ModelSlot::Default,
            Some(&requested),
            &ModelSelectionPolicy::DeveloperAnyConfigured,
            Some(&route("anthropic", "claude-sonnet")),
        )
        .unwrap();
    assert_eq!(result.slot, ModelSlot::Default);
    assert_eq!(result.route, requested);
}

#[test]
fn managed_only_rejects_user_provided_routes() {
    let resolver = InMemoryModelRouteResolver;
    let err = resolver
        .resolve_model_route(
            &ModelSlot::Default,
            Some(&route("openai", "gpt-4o")),
            &ModelSelectionPolicy::ManagedOnly,
            Some(&route("anthropic", "claude-sonnet")),
        )
        .unwrap_err();
    assert_eq!(err, ModelRouteResolutionError::RouteNotAllowed);
}

#[test]
fn user_selectable_allowlist_accepts_listed_routes() {
    let resolver = InMemoryModelRouteResolver;
    let allowed_route = route("openai", "gpt-4o");
    let result = resolver
        .resolve_model_route(
            &ModelSlot::Default,
            Some(&allowed_route),
            &ModelSelectionPolicy::UserSelectableAllowlist {
                allowed: vec![allowed_route.clone(), route("anthropic", "claude-sonnet")],
            },
            None,
        )
        .unwrap();
    assert_eq!(result.route, allowed_route);
}

#[test]
fn user_selectable_allowlist_rejects_unlisted_routes() {
    let resolver = InMemoryModelRouteResolver;
    let err = resolver
        .resolve_model_route(
            &ModelSlot::Default,
            Some(&route("ollama", "llama3")),
            &ModelSelectionPolicy::UserSelectableAllowlist {
                allowed: vec![route("openai", "gpt-4o")],
            },
            None,
        )
        .unwrap_err();
    assert_eq!(err, ModelRouteResolutionError::RouteNotAllowed);
}

#[test]
fn missing_unconfigured_slot_returns_slot_unconfigured() {
    let resolver = InMemoryModelRouteResolver;
    let err = resolver
        .resolve_model_route(
            &ModelSlot::Default,
            None,
            &ModelSelectionPolicy::DeveloperAnyConfigured,
            None,
        )
        .unwrap_err();
    assert_eq!(err, ModelRouteResolutionError::SlotUnconfigured);
}

#[test]
fn resolved_model_route_round_trips_through_serde() {
    let resolved = ResolvedModelRoute {
        slot: ModelSlot::Default,
        route: route("openai", "gpt-4o"),
    };
    let json = serde_json::to_string(&resolved).unwrap();
    let deserialized: ResolvedModelRoute = serde_json::from_str(&json).unwrap();
    assert_eq!(resolved, deserialized);
}

#[test]
fn provider_id_and_model_id_validate_on_construction() {
    assert!(ProviderId::new("valid-provider").is_ok());
    assert!(ProviderId::new("INVALID PROVIDER").is_err());
    assert!(ProviderId::new("").is_err());
    assert!(ModelId::new("gpt-4o").is_ok());
    assert!(ModelId::new("BAD MODEL").is_err());
}

#[test]
fn managed_only_uses_configured_default_when_no_request() {
    let resolver = InMemoryModelRouteResolver;
    let default = route("anthropic", "claude-sonnet");
    let result = resolver
        .resolve_model_route(
            &ModelSlot::Default,
            None,
            &ModelSelectionPolicy::ManagedOnly,
            Some(&default),
        )
        .unwrap();
    assert_eq!(result.slot, ModelSlot::Default);
    assert_eq!(result.route, default);
}
