mod support;

use support::legacy_capability_fixture_to_v2_with_schema_suffix as legacy_capability_fixture_to_v2;

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, SurfaceKind, VisibleCapabilityAccess,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustError, TrustPolicy, TrustPolicyInput, TrustProvenance,
};
use serde_json::json;

#[tokio::test]
async fn visible_surface_empty_registry_returns_deterministic_empty_version() {
    let runtime = runtime_with(ExtensionRegistry::new(), Arc::new(GrantAuthorizer));
    let context = context_with_grants([]);
    let request = VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap());

    let first = runtime.visible_capabilities(request.clone()).await.unwrap();
    let second = runtime.visible_capabilities(request).await.unwrap();

    assert!(first.capabilities.is_empty());
    assert_eq!(first.version, second.version);
    assert_ne!(first.version.as_str(), "surface-v1");
    assert!(first.version.as_str().starts_with("sha256:"));
}

#[tokio::test]
async fn visible_surface_default_policy_and_missing_provider_trust_fail_closed() {
    let authorizer = Arc::new(CountingGrantAuthorizer::default());
    let runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        authorizer.clone(),
    );
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);

    let default_policy_surface = runtime
        .visible_capabilities(
            VisibleCapabilityRequest::new(context.clone(), SurfaceKind::new("agent_loop").unwrap())
                .with_provider_trust(provider_trust_for(default_provider_trust())),
        )
        .await
        .unwrap();
    assert!(default_policy_surface.capabilities.is_empty());
    assert_eq!(authorizer.call_count(), 0);

    let missing_trust_surface = runtime
        .visible_capabilities(
            VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
                .with_policy(CapabilitySurfacePolicy::allow_all()),
        )
        .await
        .unwrap();
    assert!(missing_trust_surface.capabilities.is_empty());
    assert_eq!(authorizer.call_count(), 0);
}

#[tokio::test]
async fn visible_surface_uses_caller_provider_trust_not_host_trust_policy() {
    let runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(PanicTrustPolicy));
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);

    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_eq!(visible_ids(&surface), vec![capability_id("echo.say")]);
}

#[tokio::test]
async fn visible_surface_filters_by_grants_provider_trust_and_preserves_registry_order() {
    let registry = registry_from_manifests([
        (ECHO_MANIFEST, "/system/extensions/echo"),
        (FILES_MANIFEST, "/system/extensions/files"),
        (NET_MANIFEST, "/system/extensions/net"),
    ]);
    let runtime = runtime_with(registry, Arc::new(GrantAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([
            (
                "echo",
                "/system/extensions/echo/manifest.toml",
                vec![EffectKind::DispatchCapability],
            ),
            (
                "files",
                "/system/extensions/files/manifest.toml",
                vec![EffectKind::ReadFilesystem],
            ),
            (
                "net",
                "/system/extensions/net/manifest.toml",
                vec![EffectKind::Network],
            ),
        ]),
    ));
    let context = context_with_grants([
        (
            capability_id("files.read"),
            vec![EffectKind::ReadFilesystem],
        ),
        (
            capability_id("echo.say"),
            vec![EffectKind::DispatchCapability],
        ),
    ]);

    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    let visible_ids: Vec<_> = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.clone())
        .collect();
    assert_eq!(
        visible_ids,
        vec![capability_id("echo.say"), capability_id("files.read")],
        "filtered surface must preserve registry order, not grant order"
    );
    assert_eq!(surface.capabilities.len(), 2);
    assert!(
        surface
            .capabilities
            .iter()
            .all(|capability| capability.access == VisibleCapabilityAccess::Available)
    );
}

#[tokio::test]
async fn visible_surface_omits_missing_trust_and_insufficient_trust_ceiling() {
    let registry = registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]);
    let granted_context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);

    let missing_policy_runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    );
    let missing_policy_surface = missing_policy_runtime
        .visible_capabilities(request_with_provider_trust(
            granted_context.clone(),
            Vec::new(),
        ))
        .await
        .unwrap();
    assert!(missing_policy_surface.capabilities.is_empty());

    let insufficient_policy_runtime = runtime_with(registry, Arc::new(GrantAuthorizer));
    let insufficient_surface = insufficient_policy_runtime
        .visible_capabilities(request_with_provider_trust(
            granted_context,
            vec![("echo", Vec::new())],
        ))
        .await
        .unwrap();
    assert!(insufficient_surface.capabilities.is_empty());
}

#[tokio::test]
async fn visible_surface_policy_filters_runtime_and_effects_before_authorization() {
    let registry = registry_from_manifests([
        (ECHO_MANIFEST, "/system/extensions/echo"),
        (SCRIPT_MANIFEST, "/system/extensions/scripts"),
        (NET_MANIFEST, "/system/extensions/net"),
    ]);
    let authorizer = Arc::new(PanicAuthorizer);
    let runtime =
        runtime_with(registry, authorizer).with_trust_policy(Arc::new(trust_policy_for([
            (
                "echo",
                "/system/extensions/echo/manifest.toml",
                vec![EffectKind::DispatchCapability],
            ),
            (
                "scripts",
                "/system/extensions/scripts/manifest.toml",
                vec![EffectKind::ExecuteCode],
            ),
            (
                "net",
                "/system/extensions/net/manifest.toml",
                vec![EffectKind::Network],
            ),
        ])));
    let mut request = visible_request(context_with_grants([
        (
            capability_id("echo.say"),
            vec![EffectKind::DispatchCapability],
        ),
        (capability_id("scripts.run"), vec![EffectKind::ExecuteCode]),
        (capability_id("net.fetch"), vec![EffectKind::Network]),
    ]));
    request.policy = CapabilitySurfacePolicy {
        allowed_runtimes: vec![RuntimeKind::Wasm],
        allowed_effects: vec![EffectKind::DispatchCapability],
        include_requires_approval: true,
        max_capabilities: None,
    };

    let surface = runtime.visible_capabilities(request).await.unwrap();

    assert_eq!(surface.capabilities.len(), 1);
    assert_eq!(
        surface.capabilities[0].descriptor.id,
        capability_id("echo.say")
    );
}

#[tokio::test]
async fn visible_surface_marks_askable_capabilities_without_granting_authority() {
    let registry = registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]);
    let runtime = runtime_with(registry, Arc::new(ApprovalAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "echo",
            "/system/extensions/echo/manifest.toml",
            vec![EffectKind::DispatchCapability],
        )]),
    ));
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);

    let surface = runtime
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();

    assert_eq!(surface.capabilities.len(), 1);
    assert_eq!(
        surface.capabilities[0].access,
        VisibleCapabilityAccess::RequiresApproval
    );

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id("echo.say"),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Failed(_)),
        "surface visibility must not bypass approval stores or grant authority"
    );
}

#[tokio::test]
async fn hidden_capability_direct_invoke_still_fails_closed_through_authorization() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let runtime = DefaultHostRuntime::new(
        Arc::new(registry_from_manifests([(
            ECHO_MANIFEST,
            "/system/extensions/echo",
        )])),
        dispatcher.clone(),
        Arc::new(GrantAuthorizer),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));
    let context = context_with_grants([]);

    let surface = runtime
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    assert!(surface.capabilities.is_empty());

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id("echo.say"),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind.as_str(), "authorization");
        }
        other => panic!("expected authorization failure, got {other:?}"),
    }
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn visible_surface_version_changes_with_schema_and_policy_changes() {
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let runtime_a = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));
    let runtime_b = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST_WITH_SCHEMA, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));

    let surface_a = runtime_a
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    let surface_b = runtime_b
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    assert_ne!(surface_a.version, surface_b.version);

    let mut policy_request = visible_request(context);
    policy_request.policy.max_capabilities = Some(0);
    let narrowed = runtime_a
        .visible_capabilities(policy_request)
        .await
        .unwrap();
    assert_ne!(surface_a.version, narrowed.version);
}

#[tokio::test]
async fn visible_surface_version_changes_with_returned_descriptor_metadata() {
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let runtime_a = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));
    let runtime_b = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST_WITH_DESCRIPTION, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));

    let surface_a = runtime_a
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    let surface_b = runtime_b
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_ne!(
        surface_a.capabilities[0].descriptor.description,
        surface_b.capabilities[0].descriptor.description
    );
    assert_ne!(
        surface_a.version, surface_b.version,
        "surface version must change when returned descriptor metadata changes"
    );
}

#[tokio::test]
async fn visible_surface_version_is_order_insensitive_for_equivalent_policy() {
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::new(trust_policy_for([(
        "echo",
        "/system/extensions/echo/manifest.toml",
        vec![EffectKind::DispatchCapability],
    )])));

    let policy_a = CapabilitySurfacePolicy {
        allowed_runtimes: vec![RuntimeKind::Wasm, RuntimeKind::Script],
        allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        include_requires_approval: true,
        max_capabilities: None,
    };
    let policy_b = CapabilitySurfacePolicy {
        allowed_runtimes: vec![RuntimeKind::Script, RuntimeKind::Wasm],
        allowed_effects: vec![EffectKind::Network, EffectKind::DispatchCapability],
        include_requires_approval: true,
        max_capabilities: None,
    };

    let surface_a = runtime
        .visible_capabilities(visible_request(context.clone()).with_policy(policy_a))
        .await
        .unwrap();
    let surface_b = runtime
        .visible_capabilities(visible_request(context).with_policy(policy_b))
        .await
        .unwrap();

    assert_eq!(visible_ids(&surface_a), visible_ids(&surface_b));
    assert_eq!(
        surface_a.version, surface_b.version,
        "equivalent allow-list ordering must not churn the surface version"
    );
}

#[tokio::test]
async fn visible_surface_version_is_order_insensitive_for_equivalent_capability_set() {
    let context = context_with_grants([
        (
            capability_id("echo.say"),
            vec![EffectKind::DispatchCapability],
        ),
        (
            capability_id("files.read"),
            vec![EffectKind::ReadFilesystem],
        ),
    ]);
    let trust_policy = Arc::new(trust_policy_for([
        (
            "echo",
            "/system/extensions/echo/manifest.toml",
            vec![EffectKind::DispatchCapability],
        ),
        (
            "files",
            "/system/extensions/files/manifest.toml",
            vec![EffectKind::ReadFilesystem],
        ),
    ]));
    let runtime_a = runtime_with(
        registry_from_manifests([
            (ECHO_MANIFEST, "/system/extensions/echo"),
            (FILES_MANIFEST, "/system/extensions/files"),
        ]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(Arc::clone(&trust_policy));
    let runtime_b = runtime_with(
        registry_from_manifests([
            (FILES_MANIFEST, "/system/extensions/files"),
            (ECHO_MANIFEST, "/system/extensions/echo"),
        ]),
        Arc::new(GrantAuthorizer),
    )
    .with_trust_policy(trust_policy);

    let surface_a = runtime_a
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    let surface_b = runtime_b
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_ne!(visible_ids(&surface_a), visible_ids(&surface_b));
    assert_eq!(
        surface_a.version, surface_b.version,
        "equivalent capability sets must hash in canonical key order"
    );
}

#[tokio::test]
async fn visible_surface_max_capabilities_stops_authorization_after_limit() {
    let registry = registry_from_manifests([
        (ECHO_MANIFEST, "/system/extensions/echo"),
        (FILES_MANIFEST, "/system/extensions/files"),
        (NET_MANIFEST, "/system/extensions/net"),
    ]);
    let authorizer = Arc::new(CountingGrantAuthorizer::default());
    let runtime =
        runtime_with(registry, authorizer.clone()).with_trust_policy(Arc::new(trust_policy_for([
            (
                "echo",
                "/system/extensions/echo/manifest.toml",
                vec![EffectKind::DispatchCapability],
            ),
            (
                "files",
                "/system/extensions/files/manifest.toml",
                vec![EffectKind::ReadFilesystem],
            ),
            (
                "net",
                "/system/extensions/net/manifest.toml",
                vec![EffectKind::Network],
            ),
        ])));
    let context = context_with_grants([
        (
            capability_id("echo.say"),
            vec![EffectKind::DispatchCapability],
        ),
        (
            capability_id("files.read"),
            vec![EffectKind::ReadFilesystem],
        ),
        (capability_id("net.fetch"), vec![EffectKind::Network]),
    ]);
    let request = visible_request(context).with_policy(CapabilitySurfacePolicy {
        max_capabilities: Some(1),
        ..CapabilitySurfacePolicy::allow_all()
    });

    let surface = runtime.visible_capabilities(request).await.unwrap();

    assert_eq!(surface.capabilities.len(), 1);
    assert_eq!(authorizer.call_count(), 1);
}

#[tokio::test]
async fn visible_surface_can_hide_approval_required_capabilities_by_policy() {
    let registry = registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]);
    let runtime = runtime_with(registry, Arc::new(ApprovalAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "echo",
            "/system/extensions/echo/manifest.toml",
            vec![EffectKind::DispatchCapability],
        )]),
    ));
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    let request = visible_request(context).with_policy(CapabilitySurfacePolicy {
        include_requires_approval: false,
        ..CapabilitySurfacePolicy::allow_all()
    });

    let surface = runtime.visible_capabilities(request).await.unwrap();

    assert!(surface.capabilities.is_empty());
}

#[tokio::test]
async fn visible_surface_requires_every_descriptor_effect_to_be_policy_allowed() {
    let registry = registry_from_manifests([(ECHO_NETWORK_MANIFEST, "/system/extensions/echo")]);
    let runtime = runtime_with(registry, Arc::new(PanicAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "echo",
            "/system/extensions/echo/manifest.toml",
            vec![EffectKind::DispatchCapability, EffectKind::Network],
        )]),
    ));
    let context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    )]);
    let request = visible_request(context).with_policy(CapabilitySurfacePolicy {
        allowed_effects: vec![EffectKind::DispatchCapability],
        ..CapabilitySurfacePolicy::allow_all()
    });

    let surface = runtime.visible_capabilities(request).await.unwrap();

    assert!(surface.capabilities.is_empty());
}

#[tokio::test]
async fn visible_surface_rejects_invalid_execution_context() {
    let runtime = runtime_with(
        registry_from_manifests([(ECHO_MANIFEST, "/system/extensions/echo")]),
        Arc::new(GrantAuthorizer),
    );
    let mut context = context_with_grants([(
        capability_id("echo.say"),
        vec![EffectKind::DispatchCapability],
    )]);
    context.resource_scope.invocation_id = InvocationId::new();

    let error = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ironclaw_host_runtime::HostRuntimeError::InvalidRequest { .. }
    ));
}

#[tokio::test]
async fn visible_surface_debug_does_not_expose_authority_internals() {
    let registry = registry_from_manifests([(SECRET_MANIFEST, "/system/extensions/secret-tool")]);
    let runtime = runtime_with(registry, Arc::new(GrantAuthorizer)).with_trust_policy(Arc::new(
        trust_policy_for([(
            "secret-tool",
            "/system/extensions/secret-tool/manifest.toml",
            vec![EffectKind::UseSecret],
        )]),
    ));
    let context = context_with_secret_grant();

    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();
    let debug = format!("{surface:?}");

    assert_eq!(surface.capabilities.len(), 1);
    assert!(!debug.contains("sentinel_secret"));
    assert!(!debug.contains("/private/sentinel"));
    assert!(!debug.contains("approval_store"));
    assert!(!debug.contains("lease"));
}

fn visible_request(context: ExecutionContext) -> VisibleCapabilityRequest {
    request_with_provider_trust(context, default_provider_trust())
}

fn request_with_provider_trust(
    context: ExecutionContext,
    provider_trust: impl IntoIterator<Item = (&'static str, Vec<EffectKind>)>,
) -> VisibleCapabilityRequest {
    VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(provider_trust_for(provider_trust))
}

fn default_provider_trust() -> Vec<(&'static str, Vec<EffectKind>)> {
    vec![
        ("echo", vec![EffectKind::DispatchCapability]),
        ("files", vec![EffectKind::ReadFilesystem]),
        ("net", vec![EffectKind::Network]),
        ("scripts", vec![EffectKind::ExecuteCode]),
        ("secret-tool", vec![EffectKind::UseSecret]),
    ]
}

fn provider_trust_for(
    entries: impl IntoIterator<Item = (&'static str, Vec<EffectKind>)>,
) -> BTreeMap<ExtensionId, TrustDecision> {
    entries
        .into_iter()
        .map(|(provider, effects)| {
            (
                ExtensionId::new(provider).unwrap(),
                trust_decision_for(effects),
            )
        })
        .collect()
}

fn trust_decision_for(allowed_effects: Vec<EffectKind>) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn visible_ids(surface: &VisibleCapabilitySurface) -> Vec<CapabilityId> {
    surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.clone())
        .collect()
}

fn runtime_with(
    registry: ExtensionRegistry,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
) -> DefaultHostRuntime {
    DefaultHostRuntime::new(
        Arc::new(registry),
        Arc::new(RecordingDispatcher::default()),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
}

fn registry_from_manifests<const N: usize>(manifests: [(&str, &str); N]) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    for (manifest, root) in manifests {
        let manifest = parse_manifest(manifest);
        let package =
            ExtensionPackage::from_manifest(manifest, VirtualPath::new(root).unwrap()).unwrap();
        registry.insert(package).unwrap();
    }
    registry
}

fn parse_manifest(manifest: &str) -> ExtensionManifest {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    ExtensionManifest::parse(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap()
}

fn trust_policy_for<const N: usize>(
    entries: [(&str, &str, Vec<EffectKind>); N],
) -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(
        entries
            .into_iter()
            .map(|(package_id, path, effects)| {
                AdminEntry::for_local_manifest(
                    PackageId::new(package_id).unwrap(),
                    path.to_string(),
                    None,
                    HostTrustAssignment::user_trusted(),
                    effects,
                    None,
                )
            })
            .collect::<Vec<_>>(),
    ))])
    .unwrap()
}

fn context_with_grants<const N: usize>(
    grants: [(CapabilityId, Vec<EffectKind>); N],
) -> ExecutionContext {
    let grants = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|(capability, allowed_effects)| CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability,
                grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects,
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            })
            .collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
}

fn context_with_secret_grant() -> ExecutionContext {
    let mut context = context_with_grants([]);
    context.grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id("secret-tool.read"),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::UseSecret],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: vec![SecretHandle::new("sentinel_secret").unwrap()],
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    context
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn trust_decision_with_dispatch_authority() -> TrustDecision {
    trust_decision_for(vec![EffectKind::DispatchCapability])
}

struct PanicTrustPolicy;

impl TrustPolicy for PanicTrustPolicy {
    fn evaluate(&self, _input: &TrustPolicyInput) -> Result<TrustDecision, TrustError> {
        panic!("visible surface must use caller-supplied provider_trust, not host trust policy")
    }
}

#[derive(Default)]
struct RecordingDispatcher {
    request: Mutex<Option<CapabilityDispatchRequest>>,
}

impl RecordingDispatcher {
    fn has_request(&self) -> bool {
        self.request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

#[async_trait]
impl CapabilityDispatcher for RecordingDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        *self
            .request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.clone());
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: ExtensionId::new("echo").unwrap(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

#[derive(Default)]
struct CountingGrantAuthorizer {
    calls: AtomicUsize,
}

impl CountingGrantAuthorizer {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for CountingGrantAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        GrantAuthorizer::new()
            .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
            .await
    }
}

struct ApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: descriptor.id.clone(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

struct PanicAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for PanicAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        if descriptor.id != capability_id("echo.say") {
            panic!("policy filters must skip authorizer for disallowed descriptors")
        }
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}

const ECHO_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;

const ECHO_NETWORK_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input over network"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = {}
"#;

const ECHO_MANIFEST_WITH_SCHEMA: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object", properties = { message = { type = "string" } } }
"#;

const ECHO_MANIFEST_WITH_DESCRIPTION: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes transformed input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;

const FILES_MANIFEST: &str = r#"
id = "files"
name = "Files"
version = "0.1.0"
description = "File reader"
trust = "third_party"

[runtime]
kind = "wasm"
module = "files.wasm"

[[capabilities]]
id = "files.read"
description = "Reads files"
effects = ["read_filesystem"]
default_permission = "allow"
parameters_schema = {}
"#;

const NET_MANIFEST: &str = r#"
id = "net"
name = "Network"
version = "0.1.0"
description = "Network fetcher"
trust = "third_party"

[runtime]
kind = "wasm"
module = "net.wasm"

[[capabilities]]
id = "net.fetch"
description = "Fetches URLs"
effects = ["network"]
default_permission = "allow"
parameters_schema = {}
"#;

const SECRET_MANIFEST: &str = r#"
id = "secret-tool"
name = "Secret Tool"
version = "0.1.0"
description = "Uses one secret"
trust = "third_party"

[runtime]
kind = "wasm"
module = "secret.wasm"

[[capabilities]]
id = "secret-tool.read"
description = "Uses a scoped secret"
effects = ["use_secret"]
default_permission = "allow"
parameters_schema = {}
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "scripts"
name = "Scripts"
version = "0.1.0"
description = "Script runner"
trust = "third_party"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "pytest"
args = ["tests/"]

[[capabilities]]
id = "scripts.run"
description = "Runs a script"
effects = ["execute_code"]
default_permission = "allow"
parameters_schema = {}
"#;
