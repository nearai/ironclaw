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
fn dispatch_errors_preserve_typed_failure_kind() {
    let capability = CapabilityId::new("test.cap").unwrap();
    let provider = ExtensionId::new("test").unwrap();

    assert_eq!(
        DispatchError::UnknownCapability {
            capability: capability.clone(),
        }
        .failure_kind(),
        DispatchFailureKind::UnknownCapability
    );
    assert_eq!(
        DispatchError::UnknownProvider {
            capability: capability.clone(),
            provider,
        }
        .failure_kind(),
        DispatchFailureKind::UnknownProvider
    );
    assert_eq!(
        DispatchError::RuntimeMismatch {
            capability: capability.clone(),
            descriptor_runtime: RuntimeKind::Wasm,
            package_runtime: RuntimeKind::Mcp,
        }
        .failure_kind(),
        DispatchFailureKind::RuntimeMismatch
    );
    assert_eq!(
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Script,
        }
        .failure_kind(),
        DispatchFailureKind::MissingRuntimeBackend
    );
    assert_eq!(
        DispatchError::UnsupportedRuntime {
            capability,
            runtime: RuntimeKind::Wasm,
        }
        .failure_kind(),
        DispatchFailureKind::UnsupportedRuntime
    );
    assert_eq!(
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
        }
        .failure_kind(),
        DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Guest)
    );
}

#[test]
fn runtime_credential_injection_rejects_missing_source() {
    let missing_source = json!({
        "handle": "api-token",
        "target": {
            "type": "header",
            "name": "authorization",
            "prefix": "Bearer "
        },
        "required": true
    });

    let error = serde_json::from_value::<RuntimeCredentialInjection>(missing_source)
        .expect_err("credential injection source is authority-bearing and must be explicit");

    assert!(
        error.to_string().contains("missing field `source`"),
        "unexpected deserialization error: {error}"
    );
}

#[test]
fn dispatch_failure_kind_display_preserves_stable_literals() {
    assert_eq!(
        DispatchFailureKind::UnknownCapability.as_str(),
        "UnknownCapability"
    );
    assert_eq!(
        DispatchFailureKind::UnknownProvider.as_str(),
        "UnknownProvider"
    );
    assert_eq!(
        DispatchFailureKind::RuntimeMismatch.as_str(),
        "RuntimeMismatch"
    );
    assert_eq!(
        DispatchFailureKind::MissingRuntimeBackend.as_str(),
        "MissingRuntimeBackend"
    );
    assert_eq!(
        DispatchFailureKind::UnsupportedRuntime.as_str(),
        "UnsupportedRuntime"
    );
    assert_eq!(
        DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::NetworkDenied).as_str(),
        "NetworkDenied"
    );
    assert_eq!(
        DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::NetworkDenied).to_string(),
        "NetworkDenied"
    );
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
fn scoped_path_redacts_all_rejected_values_in_error_display() {
    for invalid in [
        "",
        "relative/path",
        "/workspace/../../secret",
        "/workspace/has\0nul",
        "\\server\\share\\private.txt",
        "\\\\server\\share\\private.txt",
        "file:///etc/passwd",
        "https://example.com/private/file",
        "/Users/alice/project/private.txt",
        "/opt/ironclaw/project/private.txt",
        "/tmp/ironclaw/project/private.txt",
        "C:\\Users\\alice\\project\\private.txt",
        "C:/Users/alice/project/private.txt",
    ] {
        let message = ScopedPath::new(invalid).unwrap_err().to_string();
        assert!(
            invalid.is_empty() || !message.contains(invalid),
            "{invalid:?} must not be echoed in {message:?}"
        );
        assert!(
            message.contains("<redacted path>"),
            "{invalid:?} should use redacted placeholder in {message:?}"
        );
    }
}

#[test]
fn virtual_path_accepts_all_frozen_v1_roots() {
    for root in [
        "/engine",
        "/system/settings",
        "/system/extensions",
        "/system/skills",
        "/users",
        "/projects",
        "/memory",
        "/artifacts",
        "/tmp",
        "/secrets",
        "/events",
    ] {
        assert!(
            VirtualPath::new(root).is_ok(),
            "frozen V1 root {root:?} should be accepted"
        );
        let child = format!("{root}/child");
        assert!(
            VirtualPath::new(child).is_ok(),
            "children of frozen V1 root {root:?} should be accepted"
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

#[test]
fn host_port_ids_are_host_namespaced_and_serializable() {
    let http_egress = HostPortId::new(HOST_RUNTIME_HTTP_EGRESS_PORT_ID).unwrap();
    assert_eq!(http_egress.as_str(), "host.runtime.http_egress");

    let id = HostPortId::new("host.storage.sql_transaction.first_party").unwrap();
    assert_eq!(id.as_str(), "host.storage.sql_transaction.first_party");
    assert_eq!(serde_json::to_value(&id).unwrap(), json!(id.as_str()));
    assert_eq!(
        serde_json::from_value::<HostPortId>(json!(id.as_str())).unwrap(),
        id
    );

    for invalid in [
        "",
        "storage.sql_transaction",
        "host",
        "host.",
        "host..storage",
        "Host.storage",
        "host/storage",
        "host.storage\ntransaction",
        "host.x",
        "host.1.foo",
        "host.storage.1tier",
    ] {
        assert!(
            HostPortId::new(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
        assert!(
            serde_json::from_value::<HostPortId>(json!(invalid)).is_err(),
            "{invalid:?} should also be rejected when deserialized"
        );
    }
}

#[test]
fn host_port_view_rejects_duplicate_ports_and_answers_membership() {
    let storage = HostPortId::new("host.storage.sql_transaction.first_party").unwrap();
    let audit = HostPortId::new("host.events.audit").unwrap();
    let network = HostPortId::new("host.network.http").unwrap();

    let view = HostPortView::new(vec![
        HostPortGrant::new(storage.clone()),
        HostPortGrant::new(audit.clone()),
    ])
    .unwrap();

    assert!(view.allows(&storage));
    assert!(view.allows(&audit));
    assert!(!view.allows(&network));
    assert!(view.allows_all([&storage, &audit]));
    assert!(!view.allows_all([&storage, &network]));
    assert_eq!(view.grants()[0].id(), &audit);
    assert_eq!(view.grants()[1].id(), &storage);

    assert!(
        HostPortView::new(vec![
            HostPortGrant::new(storage.clone()),
            HostPortGrant::new(storage),
        ])
        .is_err(),
        "duplicate host port grants must fail closed"
    );
}

#[test]
fn host_port_catalog_equality_is_order_independent() {
    let storage = HostPortId::new("host.storage.sql_transaction.first_party").unwrap();
    let audit = HostPortId::new("host.events.audit").unwrap();

    let a = HostPortCatalog::new(vec![
        HostPortCatalogEntry::new(storage.clone()),
        HostPortCatalogEntry::new(audit.clone()),
    ])
    .unwrap();
    let b = HostPortCatalog::new(vec![
        HostPortCatalogEntry::new(audit),
        HostPortCatalogEntry::new(storage),
    ])
    .unwrap();

    assert_eq!(a, b);
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap(),
    );
}

#[test]
fn capability_profile_contract_equality_is_order_independent() {
    let profile_id = CapabilityProfileId::new("memory.context_retrieval.v1").unwrap();
    let op1 = CapabilityProfileOperationContract::new(
        CapabilityProfileOperationId::new("memory.context.retrieve.v1").unwrap(),
        "schemas/memory/context-retrieve.input.v1.json",
        "schemas/memory/context-retrieve.output.v1.json",
    )
    .unwrap();
    let op2 = CapabilityProfileOperationContract::new(
        CapabilityProfileOperationId::new("memory.context.touch.v1").unwrap(),
        "schemas/memory/context-touch.input.v1.json",
        "schemas/memory/context-touch.output.v1.json",
    )
    .unwrap();

    let a =
        CapabilityProfileContract::new(profile_id.clone(), vec![op1.clone(), op2.clone()]).unwrap();
    let b = CapabilityProfileContract::new(profile_id, vec![op2, op1]).unwrap();

    assert_eq!(a, b);
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap(),
    );
}

#[test]
fn host_api_contract_types_reject_unknown_fields_on_deserialize() {
    let storage = "host.storage.sql_transaction.first_party";
    let op_id = "memory.context.retrieve.v1";
    let profile_id = "memory.context_retrieval.v1";
    let in_ref = "schemas/memory/context-retrieve.input.v1.json";
    let out_ref = "schemas/memory/context-retrieve.output.v1.json";
    let ingress_policy = json!({
        "listener_class": "local_gateway",
        "auth": {
            "type": "required",
            "schemes": ["bearer_token"],
        },
        "scope_source": "authenticated_caller",
        "body_limit": {
            "type": "limited",
            "max_bytes": 16384,
        },
        "rate_limit": {
            "type": "limited",
            "scope": "per_caller",
            "max_requests": 30,
            "window_seconds": 60,
        },
        "cors": "same_origin_only",
        "websocket_origin": "not_applicable",
        "streaming": "none",
        "audit": "user_action",
        "effect_path": {
            "type": "product_workflow",
        },
    });

    // Happy paths still parse.
    assert!(serde_json::from_value::<HostPortGrant>(json!({ "id": storage })).is_ok());
    assert!(serde_json::from_value::<HostPortCatalogEntry>(json!({ "id": storage })).is_ok());
    assert!(
        serde_json::from_value::<HostPortCatalog>(json!({ "entries": [{ "id": storage }] }))
            .is_ok()
    );
    assert!(
        serde_json::from_value::<HostPortView>(json!({ "grants": [{ "id": storage }] })).is_ok()
    );
    assert!(
        serde_json::from_value::<CapabilityProfileOperationContract>(json!({
            "id": op_id,
            "input_schema_ref": in_ref,
            "output_schema_ref": out_ref,
        }))
        .is_ok()
    );
    assert!(
        serde_json::from_value::<CapabilityProfileContract>(json!({
            "id": profile_id,
            "required_operations": [{
                "id": op_id,
                "input_schema_ref": in_ref,
                "output_schema_ref": out_ref,
            }],
        }))
        .is_ok()
    );
    assert!(serde_json::from_value::<IngressPolicy>(ingress_policy.clone()).is_ok());
    assert!(
        serde_json::from_value::<IngressRouteDescriptor>(json!({
            "route_id": "web_chat.send",
            "method": "post",
            "route_pattern": "/api/chat/v2/messages",
            "policy": ingress_policy.clone(),
        }))
        .is_ok()
    );
    let mut ingress_policy_with_unknown = ingress_policy.clone();
    ingress_policy_with_unknown["oops"] = json!(1);

    // Unknown fields must fail closed at the wire boundary.
    assert!(serde_json::from_value::<HostPortGrant>(json!({ "id": storage, "oops": 1 })).is_err());
    assert!(
        serde_json::from_value::<HostPortCatalogEntry>(json!({ "id": storage, "oops": 1 }))
            .is_err()
    );
    assert!(
        serde_json::from_value::<HostPortCatalog>(json!({
            "entries": [{ "id": storage }],
            "oops": 1,
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<HostPortCatalog>(
            json!({ "entries": [{ "id": storage, "oops": 1 }] })
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<HostPortView>(json!({
            "grants": [{ "id": storage }],
            "oops": 1,
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<HostPortView>(json!({ "grants": [{ "id": storage, "oops": 1 }] }))
            .is_err()
    );
    assert!(
        serde_json::from_value::<CapabilityProfileOperationContract>(json!({
            "id": op_id,
            "input_schema_ref": in_ref,
            "output_schema_ref": out_ref,
            "oops": 1,
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<CapabilityProfileContract>(json!({
            "id": profile_id,
            "required_operations": [{
                "id": op_id,
                "input_schema_ref": in_ref,
                "output_schema_ref": out_ref,
            }],
            "oops": 1,
        }))
        .is_err()
    );
    assert!(serde_json::from_value::<IngressPolicy>(ingress_policy_with_unknown).is_err());
    assert!(
        serde_json::from_value::<IngressRouteDescriptor>(json!({
            "route_id": "web_chat.send",
            "method": "post",
            "route_pattern": "/api/chat/v2/messages",
            "policy": ingress_policy,
            "oops": 1,
        }))
        .is_err()
    );
}

#[test]
fn host_port_catalog_validates_required_ports_without_creating_implementations() {
    let storage = HostPortId::new("host.storage.sql_transaction.first_party").unwrap();
    let audit = HostPortId::new("host.events.audit").unwrap();
    let network = HostPortId::new("host.network.http").unwrap();

    let catalog = HostPortCatalog::new(vec![
        HostPortCatalogEntry::new(storage.clone()),
        HostPortCatalogEntry::new(audit.clone()),
    ])
    .unwrap();

    assert!(catalog.contains(&storage));
    assert!(catalog.contains(&audit));
    assert!(!catalog.contains(&network));
    catalog.validate_required([&storage, &audit]).unwrap();

    let missing = catalog.validate_required([&storage, &network]).unwrap_err();
    assert_eq!(
        missing,
        HostApiError::InvariantViolation {
            reason: "unknown host ports host.network.http".to_string()
        }
    );

    let inspector = HostPortId::new("host.network.inspector").unwrap();
    let aggregated = catalog
        .validate_required([&network, &inspector, &network])
        .unwrap_err();
    assert_eq!(
        aggregated,
        HostApiError::InvariantViolation {
            reason: "unknown host ports host.network.http, host.network.inspector".to_string()
        }
    );
    assert_eq!(
        catalog.missing_required([&network, &inspector, &network]),
        vec![network.clone(), inspector.clone()]
    );

    assert!(
        HostPortCatalog::new(vec![
            HostPortCatalogEntry::new(storage.clone()),
            HostPortCatalogEntry::new(storage),
        ])
        .is_err(),
        "duplicate host port catalog entries must fail closed"
    );
}

#[test]
fn capability_profile_ids_are_versioned_portable_contract_names() {
    let id = CapabilityProfileId::new("memory.context_retrieval.v1").unwrap();
    assert_eq!(id.as_str(), "memory.context_retrieval.v1");
    assert_eq!(serde_json::to_value(&id).unwrap(), json!(id.as_str()));
    assert_eq!(
        serde_json::from_value::<CapabilityProfileId>(json!(id.as_str())).unwrap(),
        id
    );

    for invalid in [
        "",
        "memory",
        "memory.context_retrieval",
        "memory.context_retrieval.version1",
        "Memory.context_retrieval.v1",
        "memory/context_retrieval/v1",
        "memory..context_retrieval.v1",
        "memory.context_retrieval.v1\n",
        "1memory.context_retrieval.v1",
        "memory.2context_retrieval.v1",
    ] {
        assert!(
            CapabilityProfileId::new(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
        assert!(
            serde_json::from_value::<CapabilityProfileId>(json!(invalid)).is_err(),
            "{invalid:?} should also be rejected when deserialized"
        );
    }
}

#[test]
fn capability_profile_contract_rejects_empty_or_duplicate_operations() {
    let profile_id = CapabilityProfileId::new("memory.context_retrieval.v1").unwrap();
    let operation = CapabilityProfileOperationContract::new(
        CapabilityProfileOperationId::new("memory.context.retrieve.v1").unwrap(),
        "schemas/memory/context-retrieve.input.v1.json",
        "schemas/memory/context-retrieve.output.v1.json",
    )
    .unwrap();

    let contract = CapabilityProfileContract::new(profile_id.clone(), vec![operation.clone()])
        .expect("single-operation profile is valid");
    assert_eq!(contract.id(), &profile_id);
    assert_eq!(
        contract.required_operations(),
        std::slice::from_ref(&operation)
    );

    assert!(
        CapabilityProfileContract::new(profile_id.clone(), Vec::new()).is_err(),
        "profiles without required operations should fail closed"
    );
    assert!(
        CapabilityProfileContract::new(profile_id, vec![operation.clone(), operation]).is_err(),
        "duplicate profile operation contracts should fail closed"
    );
}

#[test]
fn capability_profile_schema_refs_are_relative_repository_paths() {
    for valid in [
        "schemas/memory/context-retrieve.input.v1.json",
        "schemas/echo.output.v1.json",
    ] {
        assert!(
            CapabilityProfileSchemaRef::new(valid).is_ok(),
            "{valid:?} should be accepted"
        );
    }

    for invalid in [
        "",
        "/schemas/memory/context.json",
        "../schemas/memory/context.json",
        "schemas/../context.json",
        "https://example.com/schema.json",
        "file:///tmp/schema.json",
        "schemas/memory/context.json\n",
        "data:text/plain,evil",
        "mailto:foo",
        "javascript:alert(1)",
        "schemas/memory/with:colon.json",
        "c:/win/schema.json",
        "schemas/memory/contains space.json",
    ] {
        assert!(
            CapabilityProfileSchemaRef::new(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
    }
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
