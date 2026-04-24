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
