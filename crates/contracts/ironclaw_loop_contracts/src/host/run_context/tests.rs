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

#[test]
fn output_contract_is_snapshotted_independently_of_resolved_profile() {
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId};
    use ironclaw_host_api::output::OutputContract;
    use ironclaw_host_api::turn::{RunProfileId, RunProfileVersion};

    let scope = TurnScope::new(
        TenantId::new("tenant-output-contract").expect("tenant"),
        Some(AgentId::new("agent-output-contract").expect("agent")),
        Some(ProjectId::new("project-output-contract").expect("project")),
        ThreadId::new("thread-output-contract").expect("thread"),
    );
    let schema = OutputContract::JsonSchema {
        name: "answer_v1".to_string(),
        schema: serde_json::json!({"type": "object", "required": ["answer"]}),
    };
    let first_profile = ResolvedRunProfile::legacy_compatibility(
        RunProfileId::default_profile(),
        RunProfileVersion::new(1),
        true,
    );
    let second_profile = ResolvedRunProfile::legacy_compatibility(
        RunProfileId::interactive_default(),
        RunProfileVersion::new(1),
        true,
    );
    let first = LoopRunContext::new(
        scope.clone(),
        TurnId::new(),
        TurnRunId::new(),
        first_profile,
    )
    .with_output_contract(schema.clone());
    let second = LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), second_profile)
        .with_output_contract(schema.clone());

    assert_eq!(first.output_contract, schema);
    assert_eq!(second.output_contract, schema);
    let first_wire = serde_json::to_value(&first).expect("serialize first context");
    let second_wire = serde_json::to_value(&second).expect("serialize second context");
    assert_eq!(
        first_wire["output_contract"],
        second_wire["output_contract"]
    );
    let restored: LoopRunContext = serde_json::from_value(first_wire).expect("restore context");
    assert_eq!(restored.output_contract, schema);
}

#[test]
fn legacy_context_without_output_contract_rehydrates_assistant_default() {
    let scope = TurnScope::new(
        ironclaw_host_api::ids::TenantId::new("tenant-output-legacy").expect("tenant"),
        None,
        None,
        ThreadId::new("thread-output-legacy").expect("thread"),
    );
    let profile = ResolvedRunProfile::legacy_compatibility(
        ironclaw_host_api::turn::RunProfileId::default_profile(),
        ironclaw_host_api::turn::RunProfileVersion::new(1),
        true,
    );
    let context = LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), profile);
    let mut wire = serde_json::to_value(&context).expect("serialize context");
    wire.as_object_mut()
        .expect("context wire object")
        .remove("output_contract");
    let restored: LoopRunContext = serde_json::from_value(wire).expect("restore context");
    assert_eq!(
        restored.output_contract,
        ironclaw_host_api::output::OutputContract::AssistantMessage
    );
}

mod acting_identity_ladder {
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId};
    use ironclaw_host_api::turn::{RunProfileId, TurnThreadOwner};

    use super::*;

    fn ladder_context(owner: Option<&str>, actor: Option<&str>) -> LoopRunContext {
        let mut scope = TurnScope::new(
            TenantId::new("tenant-ladder").unwrap(),
            Some(AgentId::new("agent-ladder").unwrap()),
            Some(ProjectId::new("project-ladder").unwrap()),
            ThreadId::new("thread-ladder").unwrap(),
        );
        scope.thread_owner =
            TurnThreadOwner::explicit(owner.map(|owner| UserId::new(owner).unwrap()));
        let resolved = ResolvedRunProfile::legacy_compatibility(
            RunProfileId::interactive_default(),
            RunProfileVersion::new(1),
            true,
        );
        let context = LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved);
        match actor {
            Some(actor) => context.with_actor(TurnActor::new(UserId::new(actor).unwrap())),
            None => context,
        }
    }

    fn fallback() -> UserId {
        UserId::new("user-configured-fallback").unwrap()
    }

    /// The one contract ladder every scope-keyed gate-dance store keys by:
    /// actor first, explicit thread owner second, configured fallback last.
    #[test]
    fn acting_user_id_prefers_actor_then_owner_then_configured_fallback() {
        let actor_wins = ladder_context(Some("user-owner"), Some("user-actor"));
        assert_eq!(
            actor_wins.acting_user_id(&fallback()),
            UserId::new("user-actor").unwrap(),
            "an authenticated actor is the identity the run acts as"
        );

        let owner_when_actorless = ladder_context(Some("user-owner"), None);
        assert_eq!(
            owner_when_actorless.acting_user_id(&fallback()),
            UserId::new("user-owner").unwrap(),
            "a host-initiated run falls back to the thread's explicit owner"
        );

        let fallback_when_neither = ladder_context(None, None);
        assert_eq!(
            fallback_when_neither.acting_user_id(&fallback()),
            fallback(),
            "no actor and no explicit owner resolves to the configured fallback"
        );
    }

    /// The scope projection both gate-dance sides key their stores by: the
    /// run's resource scope with `user_id` swapped for the acting user.
    #[test]
    fn acting_resource_scope_carries_the_acting_user_over_the_run_scope() {
        let context = ladder_context(Some("user-owner"), Some("user-actor"));
        let scope = context.acting_resource_scope(&fallback());
        assert_eq!(scope.user_id, UserId::new("user-actor").unwrap());
        assert_eq!(scope.tenant_id, context.scope.tenant_id);
        assert_eq!(scope.agent_id, context.scope.agent_id);
        assert_eq!(scope.project_id, context.scope.project_id);

        let ownerless = ladder_context(None, None);
        assert_eq!(
            ownerless.acting_resource_scope(&fallback()).user_id,
            fallback(),
            "the resource scope's user follows the same three-rung ladder"
        );
    }
}
