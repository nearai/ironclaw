use std::{collections::BTreeMap, sync::Arc};

use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, CapabilitySurfaceVersion, ECHO_CAPABILITY_ID, HostRuntime,
    HostRuntimeServices, JSON_CAPABILITY_ID, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeFailureKind, SurfaceKind, TIME_CAPABILITY_ID, VisibleCapabilityAccess,
    VisibleCapabilityRequest, builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::{Value, json};

#[tokio::test]
async fn builtin_first_party_package_declares_expected_capabilities() {
    let package = builtin_first_party_package().unwrap();
    assert_eq!(package.id, provider_id());
    assert_eq!(package.manifest.runtime_kind(), RuntimeKind::FirstParty);

    let ids = package
        .capabilities
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![ECHO_CAPABILITY_ID, TIME_CAPABILITY_ID, JSON_CAPABILITY_ID]
    );

    let handlers = builtin_first_party_handlers().unwrap();
    for id in [ECHO_CAPABILITY_ID, TIME_CAPABILITY_ID, JSON_CAPABILITY_ID] {
        assert!(handlers.contains_handler(&capability_id(id)));
    }
}

#[tokio::test]
async fn builtin_first_party_surface_lists_allowed_tools_in_registry_order() {
    let runtime = runtime();
    let request = VisibleCapabilityRequest::new(
        execution_context([ECHO_CAPABILITY_ID, TIME_CAPABILITY_ID, JSON_CAPABILITY_ID]),
        SurfaceKind::new("agent_loop").unwrap(),
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust());

    let surface = runtime.visible_capabilities(request).await.unwrap();

    let ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![ECHO_CAPABILITY_ID, TIME_CAPABILITY_ID, JSON_CAPABILITY_ID]
    );
    assert!(
        surface
            .capabilities
            .iter()
            .all(|capability| capability.access == VisibleCapabilityAccess::Available)
    );
    assert!(
        surface
            .capabilities
            .iter()
            .all(|capability| capability.estimated_resources.output_bytes.is_some())
    );
}

#[tokio::test]
async fn builtin_rejects_oversized_inputs_before_dispatch() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"operation": "validate", "data": "x".repeat(1_048_577)}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected resource failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Resource);
}

#[tokio::test]
async fn builtin_rejects_oversized_outputs_before_return() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"operation": "stringify", "data": {"items": vec!["xxxxxxxx"; 80_000]}}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected output-too-large failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::OutputTooLarge);
}

#[tokio::test]
async fn builtin_echo_invokes_through_host_runtime() {
    let output = invoke(ECHO_CAPABILITY_ID, json!({"message": "hello reborn"}))
        .await
        .unwrap();
    assert_eq!(output, Value::String("hello reborn".to_string()));
}

#[tokio::test]
async fn builtin_time_parse_convert_and_diff_are_deterministic() {
    let parsed = invoke(
        TIME_CAPABILITY_ID,
        json!({"operation": "parse", "input": "2026-05-12T13:00:00Z"}),
    )
    .await
    .unwrap();
    assert_eq!(parsed["unix"], json!(1778590800));

    let converted = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "convert",
            "input": "2026-05-12T13:00:00Z",
            "to_timezone": "America/New_York"
        }),
    )
    .await
    .unwrap();
    assert_eq!(converted["output"], json!("2026-05-12T09:00:00-04:00"));

    let diff = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "diff",
            "input": "2026-05-12T13:00:00Z",
            "timestamp2": "2026-05-12T15:30:00Z"
        }),
    )
    .await
    .unwrap();
    assert_eq!(diff["minutes"], json!(150));
}

#[tokio::test]
async fn builtin_time_rejects_naive_without_timezone_and_ambiguous_local_time() {
    let missing_timezone = invoke(
        TIME_CAPABILITY_ID,
        json!({"operation": "parse", "input": "2026-05-12 13:00:00"}),
    )
    .await
    .unwrap_err();
    assert_eq!(missing_timezone, RuntimeFailureKind::InvalidInput);

    let ambiguous = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "parse",
            "input": "2026-11-01 01:30:00",
            "timezone": "America/New_York"
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(ambiguous, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_json_parse_query_stringify_and_validate_work() {
    let parsed = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "parse", "data": "{\"items\":[{\"name\":\"alpha\"}]}"}),
    )
    .await
    .unwrap();
    assert_eq!(parsed["items"][0]["name"], json!("alpha"));

    let queried = invoke(
        JSON_CAPABILITY_ID,
        json!({
            "operation": "query",
            "data": {"items":[{"name":"alpha"}]},
            "path": "items[0].name"
        }),
    )
    .await
    .unwrap();
    assert_eq!(queried, json!("alpha"));

    let valid = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "validate", "data": "{\"ok\":true}"}),
    )
    .await
    .unwrap();
    assert_eq!(valid, json!({"valid": true}));

    let stringified = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "stringify", "data": {"ok": true}}),
    )
    .await
    .unwrap();
    assert!(stringified.as_str().unwrap().contains("\"ok\": true"));
}

#[tokio::test]
async fn builtin_json_stringify_rejects_invalid_json_strings() {
    let error = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "stringify", "data": "not json"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_json_rejects_v1_tool_output_stash_refs_without_leaking_input() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({
                "operation": "parse",
                "source_tool_call_id": "call_RAW_SECRET_sk-provider-secret"
            }),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected sanitized failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
    let debug = format!("{failure:?}");
    assert!(!debug.contains("RAW_SECRET"));
    assert!(!debug.contains("sk-provider-secret"));
}

#[tokio::test]
async fn builtin_missing_grant_denies_before_handler_dispatch() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([]),
            capability_id(ECHO_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"message":"must not run"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected authorization failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
}

async fn invoke(capability: &str, input: Value) -> Result<Value, RuntimeFailureKind> {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([capability]),
            capability_id(capability),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

fn runtime() -> impl HostRuntime {
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers().unwrap()))
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn registry() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().unwrap())
        .unwrap();
    registry
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn provider_id() -> ExtensionId {
    ExtensionId::new("builtin").unwrap()
}

fn execution_context<const N: usize>(grants: [&str; N]) -> ExecutionContext {
    let capability_set = CapabilitySet {
        grants: grants.into_iter().map(dispatch_grant).collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        MountView::default(),
    )
    .unwrap()
}

fn dispatch_grant(capability: &str) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(capability),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap()
}

fn provider_trust() -> BTreeMap<ExtensionId, TrustDecision> {
    BTreeMap::from([(provider_id(), trust_decision())])
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}
