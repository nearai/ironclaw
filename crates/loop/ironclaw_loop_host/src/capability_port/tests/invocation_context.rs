use super::*;

#[tokio::test]
async fn invocation_context_rejects_same_scope_elevated_grant() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let mut context = execution_context("thread-elevated-grant");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        ExtensionId::new(run_context.loop_driver_id.as_str()).expect("valid extension id");
    context.grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id.clone(),
        grantee: Principal::Extension(loop_driver_extension),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::WriteFilesystem],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    let capability = RuntimeSurfaceCapabilitySnapshot {
        provider: ExtensionId::new("demo").expect("valid provider"),
        runtime: RuntimeKind::Wasm,
        estimate: ResourceEstimate::default(),
        safe_description: "demo capability".to_string(),
        description_trust: Default::default(),
        parameters_schema: serde_json::json!({"type":"object"}),
        effects: vec![EffectKind::ReadFilesystem],
        provider_tool_name: ProviderToolName::new("demo__echo").expect("provider tool name"),
    };

    let err = invocation_context_from_visible(VisibleInvocationContextRequest {
        base: &context,
        run_context: &run_context,
        activity_id: CapabilityActivityId::new(),
        capability_id: &capability_id,
        capability: &capability,
        trust: TrustClass::Sandbox,
        allowed_effects: &[EffectKind::ReadFilesystem],
        execution_mounts: &MountView::default(),
    })
    .expect_err("elevated grant must be rejected");

    assert_eq!(err.kind, AgentLoopHostErrorKind::Unauthorized);
}

#[tokio::test]
async fn invocation_context_preserves_host_mount_grants_without_context_mounts() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let mut context = execution_context("thread-host-mount-grant");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        ExtensionId::new(run_context.loop_driver_id.as_str()).expect("valid extension id");
    let grant_mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").expect("valid mount alias"),
        VirtualPath::new("/projects/demo").expect("valid virtual path"),
        MountPermissions::read_only(),
    )])
    .expect("valid mount view");
    context.grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id.clone(),
        grantee: Principal::Extension(loop_driver_extension),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::ReadFilesystem],
            mounts: grant_mounts.clone(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    let capability = RuntimeSurfaceCapabilitySnapshot {
        provider: ExtensionId::new("demo").expect("valid provider"),
        runtime: RuntimeKind::Wasm,
        estimate: ResourceEstimate::default(),
        safe_description: "demo capability".to_string(),
        description_trust: Default::default(),
        parameters_schema: serde_json::json!({"type":"object"}),
        effects: vec![EffectKind::ReadFilesystem],
        provider_tool_name: ProviderToolName::new("demo__echo").expect("provider tool name"),
    };

    let invocation_context = invocation_context_from_visible(VisibleInvocationContextRequest {
        base: &context,
        run_context: &run_context,
        activity_id: CapabilityActivityId::new(),
        capability_id: &capability_id,
        capability: &capability,
        trust: TrustClass::Sandbox,
        allowed_effects: &[EffectKind::ReadFilesystem],
        execution_mounts: &grant_mounts,
    })
    .expect("host-issued mount grant should be preserved");

    assert_eq!(invocation_context.mounts, grant_mounts);
    assert_eq!(invocation_context.grants.grants.len(), 1);
    assert_eq!(
        invocation_context.grants.grants[0].constraints.mounts,
        grant_mounts
    );
    // The invocation context must carry the turn-run identity: run-scoped
    // policy state (coding read-before-edit) keys on it, and a dropped
    // stamp would silently collapse every run into the shared `None`
    // bucket, reopening the cross-run read-state leak.
    assert_eq!(
        invocation_context.run_id,
        Some(ironclaw_host_api::ids::RunId::from_uuid(
            run_context.run_id.as_uuid()
        )),
        "invocation context must be stamped with the loop turn-run identity"
    );
    // The loop ingress is the authoritative origin source: it seals
    // `LoopRun` explicitly so the kernel does not have to fall back to
    // reconstructing origin from `run_id`.
    assert_eq!(
        invocation_context.origin,
        Some(InvocationOrigin::LoopRun(
            ironclaw_host_api::ids::RunId::from_uuid(run_context.run_id.as_uuid())
        )),
        "loop invocation context must stamp a LoopRun origin"
    );
}

#[tokio::test]
async fn invocation_context_preserves_matching_host_scope_grant() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let mut context = execution_context("thread-host-scope-grant");
    let run_context = loop_run_context(&context).await;
    context.grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id.clone(),
        grantee: Principal::Thread(context.thread_id.clone().expect("thread id")),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::ReadFilesystem],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    let capability = RuntimeSurfaceCapabilitySnapshot {
        provider: ExtensionId::new("demo").expect("valid provider"),
        runtime: RuntimeKind::Wasm,
        estimate: ResourceEstimate::default(),
        safe_description: "demo capability".to_string(),
        description_trust: Default::default(),
        parameters_schema: serde_json::json!({"type":"object"}),
        effects: vec![EffectKind::ReadFilesystem],
        provider_tool_name: ProviderToolName::new("demo__echo").expect("provider tool name"),
    };

    let invocation_context = invocation_context_from_visible(VisibleInvocationContextRequest {
        base: &context,
        run_context: &run_context,
        activity_id: CapabilityActivityId::new(),
        capability_id: &capability_id,
        capability: &capability,
        trust: TrustClass::Sandbox,
        allowed_effects: &[EffectKind::ReadFilesystem],
        execution_mounts: &MountView::default(),
    })
    .expect("matching host scope grant should be preserved");

    assert_eq!(invocation_context.grants.grants.len(), 1);
    assert!(matches!(
        &invocation_context.grants.grants[0].grantee,
        Principal::Thread(_)
    ));
}

#[tokio::test]
async fn invocation_context_derives_extension_id_for_planned_driver_namespaced_id() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let mut context = execution_context("thread-planned-driver-id");
    let mut run_context = loop_run_context(&context).await;
    run_context.loop_driver_id =
        LoopDriverId::new("reborn:planned-default").expect("valid loop driver id");
    context.grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id.clone(),
        grantee: Principal::User(context.user_id.clone()),
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
    });
    let capability = RuntimeSurfaceCapabilitySnapshot {
        provider: ExtensionId::new("demo").expect("valid provider"),
        runtime: RuntimeKind::FirstParty,
        estimate: ResourceEstimate::default(),
        safe_description: "demo echo".to_string(),
        description_trust: Default::default(),
        parameters_schema: serde_json::json!({ "type": "object" }),
        effects: vec![EffectKind::DispatchCapability],
        provider_tool_name: ProviderToolName::new("demo_echo").expect("provider tool name"),
    };

    let invocation_context = invocation_context_from_visible(VisibleInvocationContextRequest {
        base: &context,
        run_context: &run_context,
        activity_id: CapabilityActivityId::new(),
        capability_id: &capability_id,
        capability: &capability,
        trust: TrustClass::FirstParty,
        allowed_effects: &[EffectKind::DispatchCapability],
        execution_mounts: &MountView::default(),
    })
    .expect("planned driver id should derive a valid execution principal");

    assert_eq!(
        invocation_context.extension_id,
        loop_driver_execution_extension_id(&run_context).expect("valid extension")
    );
    assert_eq!(invocation_context.grants.grants.len(), 1);
}

#[tokio::test]
async fn loop_driver_execution_extension_id_includes_digest_to_avoid_slug_collisions() {
    let context = execution_context("thread-planned-driver-collisions");
    let mut colon_context = loop_run_context(&context).await;
    colon_context.loop_driver_id =
        LoopDriverId::new("reborn:planned-default").expect("valid loop driver id");
    let mut dash_context = loop_run_context(&context).await;
    dash_context.loop_driver_id =
        LoopDriverId::new("reborn-planned-default").expect("valid loop driver id");

    let colon_id = loop_driver_execution_extension_id(&colon_context).expect("valid extension id");
    let dash_id = loop_driver_execution_extension_id(&dash_context).expect("valid extension id");

    assert_ne!(colon_id, dash_id);
    assert!(
        colon_id
            .as_str()
            .starts_with("loop-driver-reborn-planned-default-")
    );
    assert_eq!(dash_id.as_str(), "reborn-planned-default");
}

#[tokio::test]
async fn invocation_context_derives_runtime_authority_from_loop_and_surface() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let mut context = execution_context("thread-derived-authority");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        ExtensionId::new(run_context.loop_driver_id.as_str()).expect("valid extension id");
    context.extension_id = ExtensionId::new("caller-supplied").expect("valid extension id");
    context.runtime = RuntimeKind::System;
    context.trust = TrustClass::System;
    context.mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").expect("valid mount alias"),
        VirtualPath::new("/projects/demo").expect("valid virtual path"),
        MountPermissions::read_write(),
    )])
    .expect("valid mount view");
    context.grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id.clone(),
        grantee: Principal::Extension(loop_driver_extension.clone()),
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
    });
    let capability = RuntimeSurfaceCapabilitySnapshot {
        provider: ExtensionId::new("demo").expect("valid provider"),
        runtime: RuntimeKind::Script,
        estimate: ResourceEstimate::default(),
        safe_description: "demo capability".to_string(),
        description_trust: Default::default(),
        parameters_schema: serde_json::json!({"type":"object"}),
        effects: vec![EffectKind::ExecuteCode],
        provider_tool_name: ProviderToolName::new("demo__echo").expect("provider tool name"),
    };

    let invocation_context = invocation_context_from_visible(VisibleInvocationContextRequest {
        base: &context,
        run_context: &run_context,
        activity_id: CapabilityActivityId::new(),
        capability_id: &capability_id,
        capability: &capability,
        trust: TrustClass::UserTrusted,
        allowed_effects: &[EffectKind::DispatchCapability],
        execution_mounts: &MountView::default(),
    })
    .expect("context");

    assert_eq!(invocation_context.extension_id, loop_driver_extension);
    assert_eq!(invocation_context.runtime, RuntimeKind::Script);
    assert_eq!(invocation_context.trust, TrustClass::UserTrusted);
    assert_eq!(invocation_context.mounts, MountView::default());
    assert_eq!(invocation_context.grants.grants.len(), 1);
}
