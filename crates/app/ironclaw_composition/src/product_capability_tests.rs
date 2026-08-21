use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{
    action::{NetworkScheme, NetworkTargetPattern},
    capability::{
        EffectKind, PermissionMode, RuntimeCredentialRequirement,
        RuntimeCredentialRequirementSource,
    },
    http::RuntimeCredentialTarget,
    ids::{AgentId, SecretHandle, TenantId, UserId},
    mount::{MountGrant, MountPermissions},
    path::{MountAlias, VirtualPath},
    runtime::{RuntimeKind, TrustClass},
};

use super::*;

#[test]
fn product_execution_context_derives_stable_artifact_namespace_from_activity() {
    let activity_id = ActivityId::new();
    let caller = ProductSurfaceCaller {
        tenant_id: TenantId::new("product-artifact-tenant").expect("tenant"),
        user_id: UserId::new("product-artifact-user").expect("user"),
        agent_id: Some(AgentId::new("product-artifact-agent").expect("agent")),
        project_id: None,
        operator_config: false,
    };
    let skill_mount_resolver = |_scope: &ResourceScope| Ok(MountView::default());
    let context = product_execution_context(
        &caller,
        activity_id,
        None,
        &skill_mount_resolver,
        &MountView::default(),
    )
    .expect("product execution context");

    assert_eq!(
        context.artifact_namespace,
        Some(ArtifactNamespaceId::from_root_run(RunId::from_uuid(
            activity_id.as_uuid(),
        )))
    );
}

#[test]
fn product_gesture_grant_keeps_no_egress_policy_unconstrained() {
    let descriptor = descriptor_with_network(Vec::new(), Vec::new());

    let grant = product_gesture_grant(
        &descriptor,
        &ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID).unwrap(),
        MountView::default(),
    );

    assert_eq!(grant.constraints.network, NetworkPolicy::default());
}

#[test]
fn product_gesture_grant_uses_dev_wildcard_for_networked_gesture_without_targets() {
    let mut descriptor = descriptor_with_network(Vec::new(), Vec::new());
    descriptor.effects.push(EffectKind::Network);

    let grant = product_gesture_grant(
        &descriptor,
        &ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID).unwrap(),
        MountView::default(),
    );

    assert_eq!(
        grant.constraints.network,
        crate::builtin_capability_policy::dev_wildcard_network_policy()
    );
}

#[test]
fn product_gesture_grant_constrains_manifest_declared_egress() {
    let target = NetworkTargetPattern {
        scheme: Some(NetworkScheme::Https),
        host_pattern: "api.example.com".to_string(),
        port: None,
    };
    let descriptor = descriptor_with_network(vec![target.clone()], Vec::new());

    let grant = product_gesture_grant(
        &descriptor,
        &ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID).unwrap(),
        MountView::default(),
    );

    assert_eq!(grant.constraints.network.allowed_targets, vec![target]);
    assert!(grant.constraints.network.deny_private_ip_ranges);
    assert_eq!(grant.constraints.network.max_egress_bytes, None);
}

#[test]
fn product_gesture_grant_folds_credential_audience_into_egress_policy() {
    let target = NetworkTargetPattern {
        scheme: Some(NetworkScheme::Https),
        host_pattern: "oauth.example.com".to_string(),
        port: None,
    };
    let credential = RuntimeCredentialRequirement {
        handle: SecretHandle::new("oauth_token").unwrap(),
        source: RuntimeCredentialRequirementSource::SecretHandle,
        provider_scopes: Vec::new(),
        audience: target.clone(),
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    };
    let descriptor = descriptor_with_network(Vec::new(), vec![credential]);

    let grant = product_gesture_grant(
        &descriptor,
        &ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID).unwrap(),
        MountView::default(),
    );

    assert_eq!(grant.constraints.network.allowed_targets, vec![target]);
    assert!(grant.constraints.network.deny_private_ip_ranges);
    assert_eq!(
        grant.constraints.secrets,
        vec![SecretHandle::new("oauth_token").unwrap()]
    );
}

#[test]
fn product_invocation_mounts_grants_extension_lifecycle_mounts() {
    let skill_mount_resolver = |_scope: &ResourceScope| Ok(MountView::default());
    for capability in [
        EXTENSION_INSTALL_CAPABILITY_ID,
        EXTENSION_ACTIVATE_CAPABILITY_ID,
        EXTENSION_REMOVE_CAPABILITY_ID,
    ] {
        let descriptor = descriptor_with_id(capability);
        let lifecycle_mounts = crate::runtime_mounts::system_extensions_lifecycle_mount_view()
            .expect("expected extension lifecycle mounts");
        let mounts = product_invocation_mounts(
            &resource_scope(),
            Some(&descriptor),
            &skill_mount_resolver,
            &lifecycle_mounts,
        )
        .expect("extension lifecycle product mounts");

        assert_eq!(mounts, lifecycle_mounts);

        let production_lifecycle_mounts =
            crate::factory::production_system_extensions_lifecycle_mount_view()
                .expect("expected production extension lifecycle mounts");
        let production_mounts = product_invocation_mounts(
            &resource_scope(),
            Some(&descriptor),
            &skill_mount_resolver,
            &production_lifecycle_mounts,
        )
        .expect("production extension lifecycle product mounts");
        assert_eq!(production_mounts, production_lifecycle_mounts);
    }
}

#[test]
fn product_invocation_mounts_keeps_skill_mounts_scoped() {
    let scope = resource_scope();
    let descriptor = descriptor_with_id(SKILL_REMOVE_CAPABILITY_ID);
    let skill_mount_resolver =
        |scope: &ResourceScope| crate::runtime_mounts::db_backed_skill_management_mount_view(scope);
    let lifecycle_mounts = MountView::default();
    let mounts = product_invocation_mounts(
        &scope,
        Some(&descriptor),
        &skill_mount_resolver,
        &lifecycle_mounts,
    )
    .expect("skill product mounts");

    assert_eq!(
        mounts,
        crate::runtime_mounts::db_backed_skill_management_mount_view(&scope)
            .expect("expected skill mounts")
    );
}

#[test]
fn product_invocation_mounts_leaves_unclassified_capabilities_empty() {
    let descriptor = descriptor_with_id("builtin.product-gesture-test");
    let skill_mount_resolver = |_scope: &ResourceScope| Ok(MountView::default());
    let lifecycle_mounts = MountView::default();
    let mounts = product_invocation_mounts(
        &resource_scope(),
        Some(&descriptor),
        &skill_mount_resolver,
        &lifecycle_mounts,
    )
    .expect("product mounts");

    assert_eq!(mounts, MountView::default());
}

#[tokio::test]
async fn product_result_replay_returns_persisted_resolution() {
    let filesystem = scoped_product_results_filesystem();
    let scope = resource_scope();
    let invocation_id = InvocationId::new();
    let result_ref = ResultRef::from_uuid(invocation_id.as_uuid());
    let body = br#"{"status":"installed"}"#.to_vec();

    persist_product_result(
        &filesystem,
        &scope,
        result_ref,
        body.clone(),
        "capability completed",
    )
    .await
    .expect("product result persists");
    let replayed = replay_product_result(&filesystem, &scope, invocation_id)
        .await
        .expect("product result replays")
        .expect("persisted result should replay");

    let Resolution::Done(outcome) = replayed else {
        panic!("persisted product result should replay as a completed outcome");
    };
    assert_eq!(outcome.refs.result, result_ref);
    assert_eq!(outcome.refs.byte_len, body.len() as u64);
    assert_eq!(outcome.verdict, ToolVerdict::Success);
}

#[tokio::test]
async fn product_result_replay_retains_artifact_metadata() {
    let filesystem = scoped_product_results_filesystem();
    let scope = resource_scope();
    let invocation_id = InvocationId::new();
    let result_ref = ResultRef::from_uuid(invocation_id.as_uuid());
    let body = br#""bounded preview""#.to_vec();
    let artifact_ref = ironclaw_host_api::artifact::ArtifactRef::new(
        ironclaw_host_api::artifact::ArtifactId::new(11),
    );
    let output_digest = ironclaw_host_api::result_meta::OutputDigest::new(73);
    persist_product_result_with_metadata(
        &filesystem,
        &scope,
        result_ref,
        body,
        "capability completed",
        Some(ProductResultArtifactMetadata {
            artifact_ref,
            byte_len: 100_000,
            output_digest: Some(output_digest),
        }),
    )
    .await
    .expect("artifact-backed product result persists");

    let replayed = replay_product_result(&filesystem, &scope, invocation_id)
        .await
        .expect("product result replays")
        .expect("persisted result");
    let Resolution::Done(outcome) = replayed else {
        panic!("persisted product result should replay as completed");
    };
    assert_eq!(outcome.refs.byte_len, 100_000);
    assert_eq!(outcome.refs.preview_meta.artifact_ref, Some(artifact_ref));
    assert_eq!(outcome.refs.output_digest, Some(output_digest));
}

#[tokio::test]
async fn product_result_replay_preserves_completion_summary() {
    let filesystem = scoped_product_results_filesystem();
    let scope = resource_scope();
    let invocation_id = InvocationId::new();
    let result_ref = ResultRef::from_uuid(invocation_id.as_uuid());
    let body = br#"{"status":"submitted"}"#.to_vec();

    persist_product_result(&filesystem, &scope, result_ref, body, "automation started")
        .await
        .expect("product result persists");
    let replayed = replay_product_result(&filesystem, &scope, invocation_id)
        .await
        .expect("product result replays")
        .expect("persisted result should replay");

    let Resolution::Done(replayed) = replayed else {
        panic!("replayed product result should be completed");
    };
    assert_eq!(replayed.summary.as_str(), "automation started");
}

#[test]
fn completed_product_result_retains_artifact_evidence() {
    let artifact_ref = ironclaw_host_api::artifact::ArtifactRef::new(
        ironclaw_host_api::artifact::ArtifactId::new(7),
    );
    let digest = ironclaw_host_api::result_meta::OutputDigest::new(41);
    let completed = ironclaw_host_runtime::RuntimeCapabilityCompleted {
        capability_id: CapabilityId::new("demo.large").expect("capability"),
        output: serde_json::Value::String("bounded preview".to_string()),
        display_preview: None,
        usage: ironclaw_host_api::resource::ResourceUsage::default().set_output_bytes(100_000),
        receipt: None,
        completed_artifact: Some(ironclaw_host_api::artifact::CompletedArtifact {
            artifact_ref,
            byte_len: 100_000,
            total_lines: Some(1),
            content_type: "application/json".to_string(),
            digest: ironclaw_host_api::artifact::ArtifactDigest::from_bytes(b"full product result"),
        }),
        canonical_output_digest: Some(digest),
        canonical_item_count: None,
    };

    let refs = product_outcome_refs(
        ResultRef::from_uuid(InvocationId::new().as_uuid()),
        "bounded preview".len(),
        completed_artifact_metadata(&completed),
    );
    assert_eq!(refs.byte_len, 100_000);
    assert_eq!(refs.preview_meta.artifact_ref, Some(artifact_ref));
    assert_eq!(refs.output_digest, Some(digest));
}

#[tokio::test]
async fn failed_runtime_outcome_inlines_full_model_visible_cause() {
    let cause = "failed reading /workspace/project/config.json";
    let outcome = RuntimeCapabilityOutcome::Failed(
        ironclaw_host_runtime::RuntimeCapabilityFailure::new(
            CapabilityId::new("demo.read").unwrap(),
            FailureKind::Backend,
            Some("capability invocation failed".to_string()),
        )
        .with_model_visible_cause(cause),
    );

    let resolution = product_resolution(
        &empty_product_result_filesystem(),
        &resource_scope(),
        InvocationId::new(),
        outcome,
    )
    .await
    .expect("runtime failure remains model-recoverable");

    assert_eq!(model_visible_failure_text(&resolution), cause);
}

#[tokio::test]
async fn missing_runtime_failure_detail_uses_explicit_fallback() {
    let failed =
        RuntimeCapabilityOutcome::Failed(ironclaw_host_runtime::RuntimeCapabilityFailure::new(
            CapabilityId::new("demo.read").unwrap(),
            FailureKind::Backend,
            Some("capability invocation failed".to_string()),
        ));
    let failed_resolution = product_resolution(
        &empty_product_result_filesystem(),
        &resource_scope(),
        InvocationId::new(),
        failed,
    )
    .await
    .expect("runtime failure remains model-recoverable");
    assert_eq!(
        model_visible_failure_text(&failed_resolution),
        "capability invocation failed"
    );
}

fn model_visible_failure_text(resolution: &Resolution) -> &str {
    let Resolution::Done(outcome) = resolution else {
        panic!("expected recoverable failure outcome, got {resolution:?}");
    };
    outcome
        .verdict
        .diagnostic()
        .and_then(ModelFailureDiagnostic::model_visible_text)
        .expect("recoverable failure must carry model-visible text")
}

fn empty_product_result_filesystem() -> ProductResultFilesystem {
    ProductResultFilesystem::Composite(crate::wrap_scoped(Arc::new(CompositeRootFilesystem::new())))
}

fn descriptor_with_id(id: &str) -> CapabilityDescriptor {
    let mut descriptor = descriptor_with_network(Vec::new(), Vec::new());
    descriptor.id = CapabilityId::new(id).unwrap();
    descriptor
}

fn resource_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: ironclaw_host_api::ids::TenantId::new("tenant-test").unwrap(),
        user_id: ironclaw_host_api::ids::UserId::new("user-test").unwrap(),
        agent_id: Some(ironclaw_host_api::ids::AgentId::new("agent-test").unwrap()),
        project_id: Some(ironclaw_host_api::ids::ProjectId::new("project-test").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn descriptor_with_network(
    network_targets: Vec<NetworkTargetPattern>,
    runtime_credentials: Vec<RuntimeCredentialRequirement>,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new("builtin.product-gesture-test").unwrap(),
        provider: ExtensionId::new("builtin").unwrap(),
        runtime: RuntimeKind::FirstParty,
        trust_ceiling: TrustClass::UserTrusted,
        description: "product gesture test".to_string(),
        parameters_schema: serde_json::json!({}),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Allow,
        runtime_credentials,
        network_targets,
        max_egress_bytes: None,
        resource_profile: None,
        origin_gate_matrix: None,
        standard_op: None,
        provider_tool_name: None,
    }
}

fn scoped_product_results_filesystem() -> ScopedFilesystem<InMemoryBackend> {
    ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        MountView::new(vec![MountGrant::new(
            MountAlias::new(PRODUCT_RESULT_ROOT).unwrap(),
            VirtualPath::new(PRODUCT_RESULT_ROOT).unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("product results mount view"),
    )
}
