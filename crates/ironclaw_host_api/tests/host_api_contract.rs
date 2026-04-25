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
    }
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
fn serde_deserialization_enforces_validated_newtype_invariants() {
    assert!(serde_json::from_value::<ExtensionId>(json!("../evil")).is_err());
    assert!(serde_json::from_value::<CapabilityId>(json!("github..search")).is_err());
    assert!(serde_json::from_value::<TenantId>(json!("tenant/name")).is_err());
    assert!(serde_json::from_value::<ScopedPath>(json!("/workspace/../../secret")).is_err());
    assert!(serde_json::from_value::<VirtualPath>(json!("/unknown/root")).is_err());
    assert!(serde_json::from_value::<MountAlias>(json!("relative")).is_err());

    let valid: ExtensionId = serde_json::from_value(json!("github-mcp.v1")).unwrap();
    assert_eq!(valid.as_str(), "github-mcp.v1");
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
fn network_targets_and_policies_are_fail_closed_by_default() {
    let deny_all = NetworkPolicy::default();
    assert!(deny_all.allowed_targets.is_empty());
    assert!(deny_all.deny_private_ip_ranges);

    assert!(NetworkTarget::new(NetworkScheme::Https, "api.github.com", Some(443)).is_ok());
    assert!(NetworkTarget::new(NetworkScheme::Https, "LOCALHOST", None).is_err());
    assert!(NetworkTarget::new(NetworkScheme::Https, "localhost", None).is_err());
    assert!(NetworkTarget::new(NetworkScheme::Https, "169.254.169.254", None).is_err());
    assert!(NetworkTarget::new(NetworkScheme::Https, "api.github.com/path", None).is_err());
    assert!(NetworkTarget::new(NetworkScheme::Https, "*.github.com", None).is_err());

    assert!(NetworkTargetPattern::new(Some(NetworkScheme::Https), "api.github.com", None).is_ok());
    assert!(NetworkTargetPattern::new(Some(NetworkScheme::Https), "*.github.com", None).is_ok());
    assert!(NetworkTargetPattern::new(Some(NetworkScheme::Https), "*.*.github.com", None).is_err());
    assert!(NetworkTargetPattern::new(Some(NetworkScheme::Https), "github.com*", None).is_err());
    assert!(NetworkTargetPattern::new(Some(NetworkScheme::Https), "localhost", None).is_err());
}

#[test]
fn network_target_deserialization_enforces_validation() {
    assert!(
        serde_json::from_value::<NetworkTarget>(json!({
            "scheme": "https",
            "host": "api.github.com",
            "port": 443
        }))
        .is_ok()
    );

    assert!(
        serde_json::from_value::<NetworkTarget>(json!({
            "scheme": "https",
            "host": "localhost",
            "port": null
        }))
        .is_err()
    );

    assert!(
        serde_json::from_value::<NetworkTargetPattern>(json!({
            "scheme": "https",
            "host_pattern": "github.com*",
            "port": null
        }))
        .is_err()
    );
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
fn mount_view_denies_unknown_alias_and_broader_child_permissions() {
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
fn invocation_fingerprint_is_stable_and_input_redacted() {
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
            project_id: Some(project_id),
            mission_id: None,
            thread_id: None,
            invocation_id,
        },
    }
}

#[test]
fn error_kind_sanitizes_detail_like_values() {
    assert_eq!(ErrorKind::new("Dispatch").as_str(), "Dispatch");
    assert_eq!(
        ErrorKind::new("failed at /tmp/secret-token.txt").as_str(),
        "Unclassified"
    );
}

#[test]
fn capability_dispatch_error_uses_host_safe_failure_kind() {
    let error = CapabilityDispatchError::new(
        CapabilityDispatchFailureKind::MissingRuntimeBackend,
        CapabilityId::new("echo.say").unwrap(),
        Some(ExtensionId::new("echo").unwrap()),
        Some(RuntimeKind::Wasm),
    );

    assert_eq!(
        error.kind,
        CapabilityDispatchFailureKind::MissingRuntimeBackend
    );
    assert_eq!(error.error_kind().as_str(), "MissingRuntimeBackend");
}
