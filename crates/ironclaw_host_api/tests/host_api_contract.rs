use std::path::PathBuf;

use ironclaw_host_api::*;
use rust_decimal_macros::dec;
use serde_json::json;

#[test]
fn extension_id_rejects_path_like_or_uppercase_values() {
    assert!(ExtensionId::new("github").is_ok());
    assert!(ExtensionId::new("github-mcp.v1").is_ok());

    for invalid in [
        "",
        "GitHub",
        "../github",
        "github/search",
        "github\\search",
        "github search",
        "github\0search",
        "github..search",
    ] {
        assert!(
            ExtensionId::new(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
    }
}

#[test]
fn capability_id_requires_extension_prefixed_name() {
    let id = CapabilityId::new("github.search_issues").unwrap();
    assert_eq!(id.as_str(), "github.search_issues");

    let nested = CapabilityId::new("github.issues.search").unwrap();
    assert_eq!(nested.as_str(), "github.issues.search");

    for invalid in [
        "github",
        "github.",
        ".search",
        "GitHub.search",
        "github/search",
        "github..search",
    ] {
        assert!(
            CapabilityId::new(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
        assert!(
            serde_json::from_value::<CapabilityId>(json!(invalid)).is_err(),
            "{invalid:?} should also be rejected when deserialized"
        );
    }
}

#[test]
fn scope_ids_reject_path_segments_and_controls() {
    assert!(TenantId::new("tenant_123").is_ok());
    assert!(UserId::new("user-123").is_ok());

    for invalid in [
        "",
        ".",
        "..",
        "user/name",
        "user\\name",
        "user\nname",
        "user\0name",
    ] {
        assert!(
            UserId::new(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
        assert!(
            serde_json::from_value::<UserId>(json!(invalid)).is_err(),
            "{invalid:?} should also be rejected when deserialized"
        );
    }
}

#[test]
fn local_default_resource_scope_uses_default_agent_and_bootstrap_project() {
    let invocation_id = InvocationId::new();
    let scope = ResourceScope::local_default(UserId::new("alice").unwrap(), invocation_id).unwrap();

    assert_eq!(LOCAL_DEFAULT_TENANT_ID, "default");
    assert_eq!(LOCAL_DEFAULT_AGENT_ID, "default");
    assert_eq!(LOCAL_DEFAULT_PROJECT_ID, "bootstrap");
    assert_eq!(scope.tenant_id.as_str(), LOCAL_DEFAULT_TENANT_ID);
    assert_eq!(scope.user_id.as_str(), "alice");
    assert_eq!(
        scope.agent_id.as_ref().map(AgentId::as_str),
        Some(LOCAL_DEFAULT_AGENT_ID)
    );
    assert_eq!(
        scope.project_id.as_ref().map(ProjectId::as_str),
        Some(LOCAL_DEFAULT_PROJECT_ID)
    );
    assert_eq!(scope.invocation_id, invocation_id);
    assert!(scope.mission_id.is_none());
    assert!(scope.thread_id.is_none());
}

#[test]
fn runtime_dispatch_error_kinds_have_safe_event_tokens() {
    for (kind, token) in [
        (RuntimeDispatchErrorKind::Backend, "backend"),
        (RuntimeDispatchErrorKind::Client, "client"),
        (RuntimeDispatchErrorKind::Executor, "executor"),
        (RuntimeDispatchErrorKind::ExitFailure, "exit_failure"),
        (
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch,
            "extension.runtime_mismatch",
        ),
        (
            RuntimeDispatchErrorKind::FilesystemDenied,
            "filesystem_denied",
        ),
        (RuntimeDispatchErrorKind::Guest, "guest"),
        (RuntimeDispatchErrorKind::InputEncode, "input_encode"),
        (RuntimeDispatchErrorKind::InvalidResult, "invalid_result"),
        (RuntimeDispatchErrorKind::Manifest, "manifest"),
        (RuntimeDispatchErrorKind::Memory, "memory"),
        (RuntimeDispatchErrorKind::MethodMissing, "method_missing"),
        (RuntimeDispatchErrorKind::NetworkDenied, "network_denied"),
        (RuntimeDispatchErrorKind::OutputDecode, "output_decode"),
        (RuntimeDispatchErrorKind::OutputTooLarge, "output_too_large"),
        (RuntimeDispatchErrorKind::Resource, "resource"),
        (
            RuntimeDispatchErrorKind::UndeclaredCapability,
            "undeclared_capability",
        ),
        (
            RuntimeDispatchErrorKind::UnsupportedRunner,
            "unsupported_runner",
        ),
        (RuntimeDispatchErrorKind::Unknown, "unknown"),
    ] {
        assert_eq!(kind.event_kind(), token);
        assert_safe_runtime_event_token(token);
    }
}

fn assert_safe_runtime_event_token(token: &str) {
    assert!(!token.is_empty(), "runtime event token must not be empty");
    assert!(
        token.len() <= 64,
        "{token:?} must fit runtime event sanitizer length"
    );
    assert!(
        token.as_bytes()[0].is_ascii_lowercase(),
        "{token:?} must start with lowercase ASCII"
    );
    assert!(
        token.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b':')
        }),
        "{token:?} must stay compatible with runtime event sanitization"
    );
    for segment in token.split(['.', ':']) {
        assert!(
            !segment.is_empty(),
            "{token:?} must not have empty segments"
        );
        assert!(
            segment.len() <= 24,
            "{token:?} segment {segment:?} must fit runtime event sanitizer segment length"
        );
        assert!(
            segment.as_bytes()[0].is_ascii_lowercase(),
            "{token:?} segment {segment:?} must start with lowercase ASCII"
        );
    }
}

#[test]
fn local_default_execution_context_keeps_scope_fields_aligned() {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/bootstrap").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    let ctx = ExecutionContext::local_default(
        UserId::new("alice").unwrap(),
        ExtensionId::new("echo").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::Sandbox,
        CapabilitySet::default(),
        mounts,
    )
    .unwrap();

    ctx.validate().unwrap();
    assert_eq!(ctx.tenant_id.as_str(), LOCAL_DEFAULT_TENANT_ID);
    assert_eq!(
        ctx.agent_id.as_ref().map(AgentId::as_str),
        Some(LOCAL_DEFAULT_AGENT_ID)
    );
    assert_eq!(
        ctx.project_id.as_ref().map(ProjectId::as_str),
        Some(LOCAL_DEFAULT_PROJECT_ID)
    );
    assert_eq!(ctx.resource_scope.tenant_id, ctx.tenant_id);
    assert_eq!(ctx.resource_scope.user_id, ctx.user_id);
    assert_eq!(ctx.resource_scope.agent_id, ctx.agent_id);
    assert_eq!(ctx.resource_scope.project_id, ctx.project_id);
}

#[test]
fn scoped_path_rejects_raw_host_paths_urls_and_traversal() {
    assert!(ScopedPath::new("/workspace/README.md").is_ok());
    assert!(ScopedPath::new("/extension/state/db.json").is_ok());

    for invalid in [
        "relative/path",
        "/workspace/../../secret",
        "file:///etc/passwd",
        "https://example.com/file",
        "/Users/alice/project",
        "/opt/ironclaw/project",
        "/tmp/ironclaw/project",
        "C:\\Users\\alice\\project",
        "/workspace/has\0nul",
    ] {
        assert!(
            ScopedPath::new(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
    }
}

#[test]
fn virtual_path_requires_known_root_and_rejects_traversal() {
    assert!(VirtualPath::new("/projects/p1/threads/t1").is_ok());
    assert!(VirtualPath::new("/system/extensions/echo/state").is_ok());

    for invalid in [
        "/unknown/root",
        "relative",
        "/projects/../users/u1",
        "file:///projects/p1",
    ] {
        assert!(
            VirtualPath::new(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
    }
}

#[test]
fn host_path_debug_redacts_and_host_path_is_not_serializable() {
    static_assertions::assert_not_impl_any!(HostPath: serde::Serialize);

    let debug = format!(
        "{:?}",
        HostPath::from_path_buf(PathBuf::from("/Users/alice/private-secret"))
    );
    assert_eq!(debug, "HostPath(<redacted>)");
    assert!(!debug.contains("alice"));
    assert!(!debug.contains("private-secret"));
}

#[test]
fn mount_view_resolves_longest_alias_match() {
    let view = MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/p1").unwrap(),
            MountPermissions::read_only(),
        ),
        MountGrant::new(
            MountAlias::new("/workspace/docs").unwrap(),
            VirtualPath::new("/projects/p1/documentation").unwrap(),
            MountPermissions::read_write(),
        ),
    ])
    .unwrap();

    let resolved = view
        .resolve(&ScopedPath::new("/workspace/docs/intro.md").unwrap())
        .unwrap();
    assert_eq!(resolved.as_str(), "/projects/p1/documentation/intro.md");

    let resolved = view
        .resolve(&ScopedPath::new("/workspace/src/lib.rs").unwrap())
        .unwrap();
    assert_eq!(resolved.as_str(), "/projects/p1/src/lib.rs");
}

#[test]
fn mount_view_denies_unknown_alias_broader_permissions_and_narrower_targets() {
    let parent = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/p1").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();

    assert!(
        parent
            .resolve(&ScopedPath::new("/memory/note.md").unwrap())
            .is_err()
    );

    let child = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/p1").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();

    assert!(!child.is_subset_of(&parent));

    let narrower_child = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/p1/subdir").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();
    assert!(!narrower_child.is_subset_of(&parent));
}

#[test]
fn mount_view_traversal_is_rejected_before_or_during_resolution() {
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/p1").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();

    assert!(ScopedPath::new("/workspace/../secret").is_err());

    assert!(serde_json::from_value::<ScopedPath>(json!("/workspace/../secret")).is_err());
    assert!(
        view.resolve(&ScopedPath::new("/workspace/file.txt").unwrap())
            .is_ok()
    );
}

#[test]
fn execution_context_validation_rejects_mismatched_resource_scope() {
    let ctx = sample_context();
    assert!(ctx.validate().is_ok());

    let mut mismatched = ctx.clone();
    mismatched.resource_scope.user_id = UserId::new("other_user").unwrap();
    assert!(mismatched.validate().is_err());
}

#[test]
fn agent_id_is_first_class_optional_execution_scope() {
    let mut ctx = sample_context_with_agent(Some("agent1"));
    assert!(ctx.validate().is_ok());
    assert_eq!(ctx.agent_id.as_ref().unwrap().as_str(), "agent1");
    assert_eq!(ctx.resource_scope.agent_id, ctx.agent_id);

    ctx.resource_scope.agent_id = Some(AgentId::new("other-agent").unwrap());
    assert!(ctx.validate().is_err());
}

#[test]
fn audit_envelope_carries_agent_scope_without_leaking_payloads() {
    let ctx = sample_context_with_agent(Some("agent1"));
    let action = Action::WriteFile {
        path: ScopedPath::new("/workspace/secret.txt").unwrap(),
        bytes: Some(12),
    };
    let envelope = AuditEnvelope::denied(
        &ctx,
        AuditStage::Denied,
        ActionSummary::from_action(&action),
        DenyReason::MissingGrant,
    );

    assert_eq!(envelope.agent_id, Some(AgentId::new("agent1").unwrap()));
    assert_eq!(
        envelope.action.target.as_deref(),
        Some("/workspace/secret.txt")
    );
    let json = serde_json::to_value(&envelope).unwrap();
    assert_eq!(json["agent_id"], "agent1");
    let serialized = serde_json::to_string(&json).unwrap();
    assert!(serialized.contains("/workspace/secret.txt"));
    assert!(!serialized.contains("/Users/alice"));
    assert!(json.get("host_path").is_none());
}

#[test]
fn invocation_fingerprint_changes_when_agent_scope_changes() {
    let capability = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let input = json!({"message":"same"});
    let agent_a = sample_context_with_agent(Some("agent-a"));
    let agent_b = sample_context_with_agent(Some("agent-b"));

    let first = InvocationFingerprint::for_dispatch(
        &agent_a.resource_scope,
        &capability,
        &estimate,
        &input,
    )
    .unwrap();
    let second = InvocationFingerprint::for_dispatch(
        &agent_b.resource_scope,
        &capability,
        &estimate,
        &input,
    )
    .unwrap();

    assert_ne!(first, second);
}

#[test]
fn principal_agent_serializes_as_first_class_principal() {
    let principal = Principal::Agent(AgentId::new("agent-a").unwrap());
    let json = serde_json::to_value(&principal).unwrap();

    assert_eq!(json, json!({"type":"agent","id":"agent-a"}));
}

#[test]
fn invocation_fingerprint_is_stable_and_input_hashed() {
    let ctx = sample_context();
    let capability = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        output_bytes: Some(10_000),
        ..ResourceEstimate::default()
    };
    let input = json!({"message": "secret payload"});
    let mut reordered = serde_json::Map::new();
    reordered.insert("z".to_string(), json!(1));
    reordered.insert("a".to_string(), json!({"b": 2, "a": 1}));

    let first =
        InvocationFingerprint::for_dispatch(&ctx.resource_scope, &capability, &estimate, &input)
            .unwrap();
    let second = InvocationFingerprint::for_dispatch(
        &ctx.resource_scope,
        &capability,
        &estimate,
        &json!({"message": "secret payload"}),
    )
    .unwrap();
    let canonical_first = InvocationFingerprint::for_dispatch(
        &ctx.resource_scope,
        &capability,
        &estimate,
        &serde_json::Value::Object(reordered),
    )
    .unwrap();
    let canonical_second = InvocationFingerprint::for_dispatch(
        &ctx.resource_scope,
        &capability,
        &estimate,
        &json!({"a": {"a": 1, "b": 2}, "z": 1}),
    )
    .unwrap();

    assert_eq!(first, second);
    assert_eq!(canonical_first, canonical_second);
    assert!(first.as_str().starts_with("sha256:"));
    assert!(!first.as_str().contains("secret payload"));
}

#[test]
fn invocation_fingerprint_separates_dispatch_and_spawn_actions() {
    let ctx = sample_context();
    let capability = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "same"});

    let dispatch =
        InvocationFingerprint::for_dispatch(&ctx.resource_scope, &capability, &estimate, &input)
            .unwrap();
    let spawn =
        InvocationFingerprint::for_spawn(&ctx.resource_scope, &capability, &estimate, &input)
            .unwrap();

    assert_ne!(dispatch, spawn);
}

#[test]
fn invocation_fingerprint_rejects_deeply_nested_input() {
    let ctx = sample_context();
    let capability = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let mut input = serde_json::Value::String("leaf".to_string());

    for _ in 0..10_000 {
        let mut object = serde_json::Map::new();
        object.insert("a".to_string(), input);
        input = serde_json::Value::Object(object);
    }

    // serde_json::Value drops nested objects recursively; leak this intentionally
    // so the test exercises fingerprint rejection rather than Value teardown.
    let input = Box::leak(Box::new(input));

    let err =
        InvocationFingerprint::for_dispatch(&ctx.resource_scope, &capability, &estimate, input)
            .unwrap_err();

    assert!(matches!(
        err,
        HostApiError::InvariantViolation { reason }
            if reason == "canonical_json: max depth exceeded"
    ));
}

#[test]
fn invocation_fingerprint_changes_when_authorized_invocation_changes() {
    let ctx = sample_context();
    let capability = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let baseline = InvocationFingerprint::for_dispatch(
        &ctx.resource_scope,
        &capability,
        &estimate,
        &json!({"message": "one"}),
    )
    .unwrap();

    let changed_input = InvocationFingerprint::for_dispatch(
        &ctx.resource_scope,
        &capability,
        &estimate,
        &json!({"message": "two"}),
    )
    .unwrap();
    let changed_capability = InvocationFingerprint::for_dispatch(
        &ctx.resource_scope,
        &CapabilityId::new("echo.other").unwrap(),
        &estimate,
        &json!({"message": "one"}),
    )
    .unwrap();
    let mut other_scope = ctx.resource_scope.clone();
    other_scope.invocation_id = InvocationId::new();
    let changed_scope = InvocationFingerprint::for_dispatch(
        &other_scope,
        &capability,
        &estimate,
        &json!({"message": "one"}),
    )
    .unwrap();

    assert_ne!(baseline, changed_input);
    assert_ne!(baseline, changed_capability);
    assert_ne!(baseline, changed_scope);
}

#[test]
fn actions_and_decisions_serialize_with_stable_snake_case_tags() {
    let action = Action::Dispatch {
        capability: CapabilityId::new("github.search_issues").unwrap(),
        estimated_resources: ResourceEstimate {
            usd: Some(dec!(0.01)),
            ..ResourceEstimate::default()
        },
    };
    let json = serde_json::to_value(&action).unwrap();
    assert_eq!(json["type"], "dispatch");

    let spawn = Action::SpawnCapability {
        capability: CapabilityId::new("github.watch_issues").unwrap(),
        estimated_resources: ResourceEstimate {
            concurrency_slots: Some(1),
            ..ResourceEstimate::default()
        },
    };
    let json = serde_json::to_value(&spawn).unwrap();
    assert_eq!(json["type"], "spawn_capability");
    assert_eq!(json["capability"], "github.watch_issues");
    assert!(json.get("extension_id").is_none());
    assert!(json.get("requested_capabilities").is_none());

    let decision = Decision::Deny {
        reason: DenyReason::MissingGrant,
    };
    let json = serde_json::to_value(&decision).unwrap();
    assert_eq!(json, json!({"type":"deny","reason":"missing_grant"}));
}

#[test]
fn action_summaries_use_stable_snake_case_targets() {
    let network = ActionSummary::from_action(&Action::Network {
        target: NetworkTarget {
            scheme: NetworkScheme::Https,
            host: "api.example.com".to_string(),
            port: Some(443),
        },
        method: NetworkMethod::Post,
        estimated_bytes: None,
    });
    assert_eq!(network.target.as_deref(), Some("post:api.example.com:443"));

    let secret = ActionSummary::from_action(&Action::UseSecret {
        handle: SecretHandle::new("google_oauth").unwrap(),
        mode: SecretUseMode::InjectIntoRequest,
    });
    assert_eq!(
        secret.target.as_deref(),
        Some("google_oauth:inject_into_request")
    );

    let extension = ActionSummary::from_action(&Action::ExtensionLifecycle {
        extension_id: ExtensionId::new("github").unwrap(),
        operation: ExtensionLifecycleOperation::Install,
    });
    assert_eq!(extension.target.as_deref(), Some("github:install"));
}

#[test]
fn obligations_are_unique_and_canonicalized() {
    let reservation_id = ResourceReservationId::new();
    let ceiling = ResourceCeiling {
        max_usd: None,
        max_input_tokens: Some(10),
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: Some(2048),
        sandbox: None,
    };
    let obligations = Obligations::new(vec![
        Obligation::AuditAfter,
        Obligation::EnforceResourceCeiling { ceiling },
        Obligation::ReserveResources { reservation_id },
        Obligation::AuditBefore,
    ])
    .unwrap();

    assert_eq!(
        obligations
            .as_slice()
            .iter()
            .map(Obligation::kind)
            .collect::<Vec<_>>(),
        vec![
            ObligationKind::ReserveResources,
            ObligationKind::AuditBefore,
            ObligationKind::EnforceResourceCeiling,
            ObligationKind::AuditAfter,
        ]
    );

    assert!(Obligations::new(vec![Obligation::AuditBefore, Obligation::AuditBefore]).is_err());

    let duplicate_json = json!([
        {"type":"audit_before"},
        {"type":"audit_before"}
    ]);
    assert!(serde_json::from_value::<Obligations>(duplicate_json).is_err());
}

#[test]
fn privileged_runtime_and_trust_classes_cannot_be_self_asserted_from_json() {
    assert_eq!(
        serde_json::from_value::<RuntimeKind>(json!("wasm")).unwrap(),
        RuntimeKind::Wasm
    );
    assert_eq!(
        serde_json::from_value::<TrustClass>(json!("sandbox")).unwrap(),
        TrustClass::Sandbox
    );

    assert!(serde_json::from_value::<RuntimeKind>(json!("first_party")).is_err());
    assert!(serde_json::from_value::<RuntimeKind>(json!("system")).is_err());
    assert!(serde_json::from_value::<TrustClass>(json!("first_party")).is_err());
    assert!(serde_json::from_value::<TrustClass>(json!("system")).is_err());
}

#[test]
fn requested_trust_class_round_trips_all_variants() {
    // Requested trust is intentionally fully deserializable — it is *declared*
    // intent, not effective authority. Privileged-sounding variants only
    // become real after policy evaluation in ironclaw_trust.
    for (raw, expected) in [
        ("untrusted", RequestedTrustClass::Untrusted),
        ("third_party", RequestedTrustClass::ThirdParty),
        (
            "first_party_requested",
            RequestedTrustClass::FirstPartyRequested,
        ),
        ("system_requested", RequestedTrustClass::SystemRequested),
    ] {
        let parsed: RequestedTrustClass = serde_json::from_value(json!(raw)).unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(serde_json::to_value(parsed).unwrap(), json!(raw));
    }
}

#[test]
fn manifest_json_with_system_field_parses_only_into_requested_type() {
    // A manifest fragment cannot be coerced into an effective TrustClass:
    // the wire form `"system"` is rejected by TrustClass deserialization but
    // accepted as RequestedTrustClass::SystemRequested when the manifest
    // schema explicitly uses the requested form. Manifests that try to use
    // `"system"` for the *effective* slot get a compile/parse error before
    // any policy code runs.
    assert!(serde_json::from_value::<TrustClass>(json!("system")).is_err());
    assert_eq!(
        serde_json::from_value::<RequestedTrustClass>(json!("system_requested")).unwrap(),
        RequestedTrustClass::SystemRequested
    );
}

#[test]
fn package_identity_serializes_with_source_tag() {
    let identity = PackageIdentity::new(
        PackageId::new("github").unwrap(),
        PackageSource::LocalManifest {
            path: "/extensions/github/manifest.toml".to_string(),
        },
        Some("abcd1234".to_string()),
        None,
    );
    let value = serde_json::to_value(&identity).unwrap();
    assert_eq!(value["package_id"], json!("github"));
    assert_eq!(value["source"]["kind"], json!("local_manifest"));
    assert_eq!(
        value["source"]["path"],
        json!("/extensions/github/manifest.toml")
    );
    assert_eq!(value["digest"], json!("abcd1234"));
    assert!(value["signer"].is_null());

    let round_trip: PackageIdentity = serde_json::from_value(value).unwrap();
    assert_eq!(round_trip, identity);
}

#[test]
fn package_source_admin_and_bundled_have_no_extra_fields() {
    let bundled: PackageSource = serde_json::from_value(json!({"kind": "bundled"})).unwrap();
    assert_eq!(bundled, PackageSource::Bundled);
    let admin: PackageSource = serde_json::from_value(json!({"kind": "admin"})).unwrap();
    assert_eq!(admin, PackageSource::Admin);
}

#[test]
fn system_principals_distinguish_host_runtime_from_named_services() {
    assert_eq!(
        serde_json::to_value(Principal::HostRuntime).unwrap(),
        json!({"type":"host_runtime"})
    );
    assert_eq!(
        serde_json::to_value(Principal::System(
            SystemServiceId::new("heartbeat").unwrap()
        ))
        .unwrap(),
        json!({"type":"system","id":"heartbeat"})
    );
}

#[test]
fn audit_envelope_serializes_redacted_summary_shape() {
    let ctx = sample_context();
    let envelope = AuditEnvelope::denied(
        &ctx,
        AuditStage::Denied,
        ActionSummary {
            kind: "dispatch".to_string(),
            target: Some("github.search_issues".to_string()),
            effects: vec![EffectKind::DispatchCapability],
        },
        DenyReason::MissingGrant,
    );

    let json = serde_json::to_value(&envelope).unwrap();
    assert_eq!(json["stage"], "denied");
    assert_eq!(json["decision"]["reason"], "missing_grant");
    assert!(json.get("host_path").is_none());
}

fn sample_context_with_agent(agent: Option<&str>) -> ExecutionContext {
    let mut ctx = sample_context();
    let agent_id = agent.map(|id| AgentId::new(id).unwrap());
    ctx.agent_id = agent_id.clone();
    ctx.resource_scope.agent_id = agent_id;
    ctx
}

fn sample_context() -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let tenant_id = TenantId::new("tenant1").unwrap();
    let user_id = UserId::new("user1").unwrap();
    let extension_id = ExtensionId::new("echo").unwrap();
    let project_id = ProjectId::new("project1").unwrap();

    ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        agent_id: None,
        project_id: Some(project_id.clone()),
        mission_id: None,
        thread_id: None,
        extension_id,
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants: CapabilitySet::default(),
        mounts: MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap(),
        resource_scope: ResourceScope {
            tenant_id,
            user_id,
            agent_id: None,
            project_id: Some(project_id),
            mission_id: None,
            thread_id: None,
            invocation_id,
        },
    }
}
