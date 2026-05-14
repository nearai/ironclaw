use ironclaw_reborn::{
    ActiveModelRouteSettings, ModelRoute, ModelRoutePolicy, ModelRouteProviderKey,
    ModelRouteResolver, ModelSelectionMode, ModelSlot, StaticModelRouteResolver,
};

#[cfg(feature = "root-llm-provider")]
use ironclaw_llm::{LlmConfig, NearAiConfig, SessionConfig};

#[test]
fn active_llm_settings_resolve_to_default_model_route() {
    let settings =
        ActiveModelRouteSettings::new("openrouter", "anthropic/claude-sonnet-4").unwrap();

    let route = ModelRoute::from_active_settings(&settings).unwrap();

    assert_eq!(route.provider_id(), "openrouter");
    assert_eq!(route.model_id(), "anthropic/claude-sonnet-4");
}

#[cfg(feature = "root-llm-provider")]
#[test]
fn llm_config_resolves_to_default_model_route_settings() {
    let config = nearai_config("qwen3-coder");

    let settings = ActiveModelRouteSettings::from_llm_config(&config).unwrap();

    assert_eq!(settings.provider_id(), "nearai");
    assert_eq!(settings.model_id(), "qwen3-coder");
}

#[test]
fn model_slots_are_exposed_in_cli_display_order() {
    assert_eq!(ModelSlot::all(), &[ModelSlot::Default, ModelSlot::Mission]);
}

#[test]
fn model_slots_cover_builtin_interactive_and_mission_profiles() {
    let interactive_model =
        ironclaw_turns::run_profile::ModelProfileId::new("interactive_model").unwrap();
    let mission_model = ironclaw_turns::run_profile::ModelProfileId::new("mission_model").unwrap();

    assert_eq!(
        ModelSlot::from_model_profile_id(&interactive_model),
        Some(ModelSlot::Default),
    );
    assert_eq!(
        ModelSlot::from_model_profile_id(&mission_model),
        Some(ModelSlot::Mission),
    );
    assert_eq!(ModelSlot::Mission.as_str(), "mission");
}

#[test]
fn route_validation_allows_bearer_prefixed_provider_ids() {
    let route = ModelRoute::new("bearer-mini", "qwen3-coder").unwrap();

    assert_eq!(route.provider_id(), "bearer-mini");
}

#[test]
fn provider_key_includes_route_config_and_auth_versions() {
    let route = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();

    let key = ModelRouteProviderKey::new(route.clone(), "config:v2", "auth:v7").unwrap();

    assert_eq!(key.route(), &route);
    assert_eq!(key.config_version(), "config:v2");
    assert_eq!(key.auth_version(), "auth:v7");
}

#[test]
fn static_resolver_preserves_provider_key_versions() {
    let route = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();
    let provider_key = ModelRouteProviderKey::new(route.clone(), "config:v3", "auth:v9").unwrap();
    let resolver = StaticModelRouteResolver::new(ModelRoutePolicy::new(
        ModelSelectionMode::DeveloperAnyConfigured,
    ))
    .with_provider_key(ModelSlot::Default, provider_key.clone());

    let snapshot = resolver.resolve(ModelSlot::Default).unwrap();

    assert_eq!(snapshot.provider_key(), &provider_key);
    assert_eq!(snapshot.route(), &route);
}

#[test]
fn developer_policy_allows_any_configured_default_route() {
    let route = ModelRoute::new("ollama", "qwen2.5-coder:7b").unwrap();
    let resolver = StaticModelRouteResolver::new(ModelRoutePolicy::new(
        ModelSelectionMode::DeveloperAnyConfigured,
    ))
    .with_route(ModelSlot::Default, route.clone());

    let snapshot = resolver.resolve(ModelSlot::Default).unwrap();

    assert_eq!(snapshot.slot(), ModelSlot::Default);
    assert_eq!(snapshot.route(), &route);
    assert_eq!(
        snapshot.policy_mode(),
        ModelSelectionMode::DeveloperAnyConfigured
    );
}

#[test]
fn user_selectable_policy_allows_explicitly_approved_default_route() {
    let route = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();
    let resolver = StaticModelRouteResolver::new(
        ModelRoutePolicy::new(ModelSelectionMode::UserSelectableAllowlist)
            .with_approved_route(route.clone()),
    )
    .with_route(ModelSlot::Default, route.clone());

    let snapshot = resolver.resolve(ModelSlot::Default).unwrap();

    assert_eq!(snapshot.route(), &route);
    assert_eq!(
        snapshot.policy_mode(),
        ModelSelectionMode::UserSelectableAllowlist
    );
}

#[test]
fn static_resolver_rejects_route_configured_for_different_slot_only() {
    let default_route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let mission_route = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();
    let resolver = StaticModelRouteResolver::new(
        ModelRoutePolicy::new(ModelSelectionMode::UserSelectableAllowlist)
            .with_approved_route(default_route.clone())
            .with_approved_route(mission_route.clone()),
    )
    .with_route(ModelSlot::Default, default_route)
    .with_route(ModelSlot::Mission, mission_route.clone());

    let error = resolver
        .validate_model_route(ModelSlot::Default, &mission_route)
        .unwrap_err();

    assert_eq!(error.kind().as_str(), "route_not_approved");
}

#[test]
fn static_resolver_allows_shared_route_configured_for_requested_slot() {
    let shared_route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let resolver = StaticModelRouteResolver::new(
        ModelRoutePolicy::new(ModelSelectionMode::UserSelectableAllowlist)
            .with_approved_route(shared_route.clone()),
    )
    .with_route(ModelSlot::Default, shared_route.clone())
    .with_route(ModelSlot::Mission, shared_route.clone());

    let mode = resolver
        .validate_model_route(ModelSlot::Default, &shared_route)
        .unwrap();

    assert_eq!(mode, ModelSelectionMode::UserSelectableAllowlist);
}

#[test]
fn managed_policy_rejects_unapproved_default_route() {
    let route = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();
    let resolver =
        StaticModelRouteResolver::new(ModelRoutePolicy::new(ModelSelectionMode::ManagedOnly))
            .with_route(ModelSlot::Default, route);

    let error = resolver.resolve(ModelSlot::Default).unwrap_err();

    assert_eq!(error.kind().as_str(), "route_not_approved");
}

#[test]
fn managed_policy_allows_explicitly_approved_default_route() {
    let route = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();
    let resolver = StaticModelRouteResolver::new(
        ModelRoutePolicy::new(ModelSelectionMode::ManagedOnly).with_approved_route(route.clone()),
    )
    .with_route(ModelSlot::Default, route.clone());

    let snapshot = resolver.resolve(ModelSlot::Default).unwrap();

    assert_eq!(snapshot.route(), &route);
}

#[test]
fn resolved_route_snapshot_remains_stable_after_settings_change() {
    let initial = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let replacement = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();
    let policy = ModelRoutePolicy::new(ModelSelectionMode::DeveloperAnyConfigured);
    let initial_resolver = StaticModelRouteResolver::new(policy.clone())
        .with_route(ModelSlot::Default, initial.clone());
    let replacement_resolver =
        StaticModelRouteResolver::new(policy).with_route(ModelSlot::Default, replacement);

    let snapshot = initial_resolver.resolve(ModelSlot::Default).unwrap();
    let later_snapshot = replacement_resolver.resolve(ModelSlot::Default).unwrap();

    assert_eq!(snapshot.route(), &initial);
    assert_ne!(snapshot.route(), later_snapshot.route());
}

#[test]
fn route_validation_rejects_secret_like_provider_ids() {
    let error = ModelRoute::new("sk-secret-provider", "gpt-4").unwrap_err();

    assert_eq!(error.kind().as_str(), "invalid_route");
}

#[test]
fn route_validation_rejects_secret_like_model_ids() {
    let error = ModelRoute::new("openrouter", "anthropic/secret-model").unwrap_err();

    assert_eq!(error.kind().as_str(), "invalid_route");
}

#[cfg(feature = "root-llm-provider")]
fn nearai_config(model: &str) -> LlmConfig {
    LlmConfig {
        backend: "nearai".to_string(),
        session: SessionConfig::default(),
        nearai: NearAiConfig {
            model: model.to_string(),
            cheap_model: None,
            base_url: "https://private.near.ai".to_string(),
            api_key: None,
            fallback_model: None,
            max_retries: 3,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: true,
        },
        provider: None,
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 120,
        cheap_model: None,
        smart_routing_cascade: true,
        max_retries: 3,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    }
}
