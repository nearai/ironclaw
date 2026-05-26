use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
    ExecutionContext, ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant,
    MountPermissions, MountView, NetworkPolicy, NetworkTargetPattern, Principal, RuntimeKind,
    TenantId, ThreadId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, ECHO_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, SHELL_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID, SurfaceKind,
    VisibleCapabilityRequest as HostVisibleCapabilityRequest,
};
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostInputBatch, HostInputEnvelope, HostInputQueue, HostInputQueueError, HostManagedModelError,
    HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse, JsonSpawnSubagentInputCodec, LoopCapabilityInputResolver,
    LoopCapabilityResultWriter, ProductLiveCancellationProbe, RunCancellationFactory,
    RunCancellationHandle, loop_driver_execution_extension_id,
    verify_product_live_cancellation_probe,
};
use ironclaw_reborn::{
    loop_exit_applier::ThreadCheckpointLoopExitEvidencePort,
    model_routes::{ModelSelectionMode, ModelSlot},
    planned_driver_factory::default_planned_run_profile_resolver,
    runtime::{
        DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, build_product_live_planned_runtime,
    },
    subagent::{
        flavors::StaticSubagentDefinitionResolver,
        gate_resolution::BoundedSubagentGateResolutionStore,
        goal_store::InMemoryBoundedSubagentGoalStore,
    },
};
use ironclaw_reborn_composition::{
    ProductLiveCapabilityAuthorityResolver, ProductLiveCapabilityIo, ProductLiveModelRouteSettings,
    ProductLivePlannedRuntimeAdapterConfig, ProductLivePlannedRuntimeAdapterError,
    ProductLivePlannedRuntimeAdapters, ProductLiveVisibleCapabilityRequestConfig, RebornBuildInput,
    RebornServices, build_reborn_services, capability_allowlist,
    visible_capability_request_for_run,
};
use ironclaw_threads::{InMemorySessionThreadService, SessionThreadService, ThreadScope};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    CheckpointStateStore, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryTurnStateStore, LoopCheckpointStore, LoopResultRef, RunProfileResolutionRequest,
    RunProfileResolver, TurnId, TurnRunId, TurnScope, TurnStateStore,
    run_profile::{
        AgentLoopHostError, CapabilityInputRef, CapabilityInvocation, CapabilityOutcome,
        InMemoryLoopHostMilestoneSink, InstructionSafetyContext, LoopCancelReasonKind,
        LoopModelBudgetAccountant, LoopModelPolicyGuard, LoopRunContext, NoOpBudgetAccountant,
        NoOpPolicyGuard, PromptMode, ProviderToolCall, VisibleCapabilityRequest,
    },
};

#[tokio::test]
async fn capability_io_resolves_staged_inputs_and_materializes_run_scoped_results() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context("capability-io").await;
    let input_ref = io
        .stage_input(&run_context, serde_json::json!({ "text": "hello" }))
        .unwrap();

    let resolved = io
        .resolve_capability_input(&run_context, &input_ref)
        .await
        .unwrap();
    assert_eq!(resolved, serde_json::json!({ "text": "hello" }));

    let result_ref = io
        .write_capability_result(
            &run_context,
            &input_ref,
            InvocationId::new(),
            &capability_id("demo.echo"),
            serde_json::json!({ "reply": "hello" }),
        )
        .await
        .unwrap();

    assert!(
        result_ref
            .as_str()
            .starts_with(&format!("result:{}.", run_context.run_id)),
        "result refs must be scoped to the loop run: {}",
        result_ref.as_str()
    );
    assert_eq!(
        io.result_for_ref(&run_context, &result_ref).unwrap(),
        serde_json::json!({ "reply": "hello" })
    );

    io.resolve_capability_input(&run_context, &input_ref)
        .await
        .expect_err("staged input refs should be consumed on successful read");
    io.result_for_ref(&run_context, &result_ref)
        .expect_err("staged result refs should be consumed on successful read");

    io.update_capability_result(
        &run_context,
        &result_ref,
        serde_json::json!({ "reply": "terminal" }),
    )
    .await
    .unwrap();
    assert_eq!(
        io.result_for_ref(&run_context, &result_ref).unwrap(),
        serde_json::json!({ "reply": "terminal" })
    );
}

#[tokio::test]
async fn capability_io_rejects_cross_run_input_and_result_refs() {
    let io = ProductLiveCapabilityIo::default();
    let first_run = loop_run_context("capability-io-first").await;
    let second_run = loop_run_context("capability-io-second").await;
    let input_ref = io
        .stage_input(&first_run, serde_json::json!({ "text": "first" }))
        .unwrap();

    let input_error = io
        .resolve_capability_input(&second_run, &input_ref)
        .await
        .expect_err("cross-run input refs must fail closed");
    assert_eq!(
        input_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::ScopeMismatch
    );

    let result_ref = io
        .write_capability_result(
            &first_run,
            &input_ref,
            InvocationId::new(),
            &capability_id("demo.echo"),
            serde_json::json!({ "reply": "first" }),
        )
        .await
        .unwrap();
    let result_error = io
        .result_for_ref(&second_run, &result_ref)
        .expect_err("cross-run result refs must fail closed");
    assert_eq!(
        result_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::ScopeMismatch
    );
}

#[tokio::test]
async fn capability_io_prunes_refs_for_terminal_runs_without_cross_run_loss() {
    let io = ProductLiveCapabilityIo::default();
    let first_run = loop_run_context("capability-io-prune-first").await;
    let second_run = loop_run_context("capability-io-prune-second").await;
    let first_input = io
        .stage_input(&first_run, serde_json::json!({ "text": "first" }))
        .unwrap();
    let second_input = io
        .stage_input(&second_run, serde_json::json!({ "text": "second" }))
        .unwrap();
    let first_result = io
        .write_capability_result(
            &first_run,
            &first_input,
            InvocationId::new(),
            &capability_id("demo.echo"),
            serde_json::json!({ "reply": "first" }),
        )
        .await
        .unwrap();
    let second_result = io
        .write_capability_result(
            &second_run,
            &second_input,
            InvocationId::new(),
            &capability_id("demo.echo"),
            serde_json::json!({ "reply": "second" }),
        )
        .await
        .unwrap();

    io.prune_run(&first_run).unwrap();

    io.resolve_capability_input(&first_run, &first_input)
        .await
        .expect_err("terminal run input refs should be pruned");
    io.result_for_ref(&first_run, &first_result)
        .expect_err("terminal run result refs should be pruned");
    assert_eq!(
        io.resolve_capability_input(&second_run, &second_input)
            .await
            .unwrap(),
        serde_json::json!({ "text": "second" })
    );
    assert_eq!(
        io.result_for_ref(&second_run, &second_result).unwrap(),
        serde_json::json!({ "reply": "second" })
    );
}

#[tokio::test]
async fn capability_io_rejects_unstaged_run_scoped_refs() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context("capability-io-unstaged").await;
    let input_ref =
        CapabilityInputRef::new(format!("input:{}:missing", run_context.run_id)).unwrap();
    let result_ref = LoopResultRef::new(format!("result:{}.missing", run_context.run_id)).unwrap();

    let input_error = io
        .resolve_capability_input(&run_context, &input_ref)
        .await
        .expect_err("unstaged same-run input refs must fail closed");
    assert_eq!(
        input_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::InvalidInvocation
    );

    let result_error = io
        .result_for_ref(&run_context, &result_ref)
        .expect_err("unstaged same-run result refs must fail closed");
    assert_eq!(
        result_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::InvalidInvocation
    );
}

#[tokio::test]
async fn capability_io_enforces_staging_entry_and_byte_caps() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context("capability-io-bounds").await;

    for index in 0..1024 {
        io.stage_input(&run_context, serde_json::json!({ "index": index }))
            .unwrap();
    }
    let entry_error = io
        .stage_input(&run_context, serde_json::json!({ "overflow": true }))
        .expect_err("staging must enforce an entry cap");
    assert_eq!(
        entry_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::BudgetExceeded
    );

    let oversized_result = serde_json::json!("x".repeat(4 * 1024 * 1024));
    let byte_error = ProductLiveCapabilityIo::default()
        .write_capability_result(
            &run_context,
            &CapabilityInputRef::new("input:oversized-result").unwrap(),
            InvocationId::new(),
            &capability_id("demo.echo"),
            oversized_result,
        )
        .await
        .expect_err("staging must enforce a serialized-byte cap");
    assert_eq!(
        byte_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::BudgetExceeded
    );
}

#[tokio::test]
async fn capability_io_rejects_oversized_staged_input_payload() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context("capability-io-input-bytes").await;

    let oversized_input = serde_json::json!("x".repeat(4 * 1024 * 1024));
    let byte_error = io
        .stage_input(&run_context, oversized_input)
        .expect_err("input staging must enforce a serialized-byte cap");

    assert_eq!(
        byte_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::BudgetExceeded
    );
}

#[tokio::test]
async fn visible_capability_request_builder_scopes_context_to_loop_run() {
    let run_context = loop_run_context("visible-builder").await;
    let request = visible_capability_request_for_run(
        &run_context,
        ProductLiveVisibleCapabilityRequestConfig::new(
            UserId::new("user-visible-builder").unwrap(),
            RuntimeKind::FirstParty,
            TrustClass::System,
            SurfaceKind::new("agent_loop").unwrap(),
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_grants(CapabilitySet::default())
        .with_provider_trust(
            ExtensionId::new("demo").unwrap(),
            EffectiveTrustClass::user_trusted(),
        ),
    )
    .unwrap();

    assert_eq!(request.context.tenant_id, run_context.scope.tenant_id);
    assert_eq!(request.context.agent_id, run_context.scope.agent_id);
    assert_eq!(request.context.project_id, run_context.scope.project_id);
    assert_eq!(
        request.context.thread_id.as_ref(),
        Some(&run_context.thread_id)
    );
    assert_eq!(
        request.context.resource_scope.thread_id.as_ref(),
        Some(&run_context.thread_id)
    );
    assert_eq!(
        request.context.extension_id,
        loop_driver_execution_extension_id(&run_context).unwrap(),
        "visible capability requests must use the same extension principal as invocation"
    );
    assert!(
        request
            .provider_trust
            .contains_key(&ExtensionId::new("demo").unwrap())
    );
}

#[tokio::test]
async fn visible_capability_request_preserves_custom_provider_trust_decision() {
    let run_context = loop_run_context("visible-custom-trust").await;
    let provider = ExtensionId::new("custom-provider").unwrap();
    let trust_decision = TrustDecision {
        effective_trust: EffectiveTrustClass::sandbox(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::ReadFilesystem],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    };

    let request = visible_capability_request_for_run(
        &run_context,
        ProductLiveVisibleCapabilityRequestConfig::new(
            UserId::new("user-visible-custom-trust").unwrap(),
            RuntimeKind::FirstParty,
            TrustClass::System,
            SurfaceKind::new("agent_loop").unwrap(),
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_provider_trust_decision(provider.clone(), trust_decision.clone()),
    )
    .unwrap();

    assert_eq!(request.provider_trust.get(&provider), Some(&trust_decision));
}

#[tokio::test]
async fn local_dev_adapter_invokes_builtin_echo_through_host_runtime_port() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "builtin-echo-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let run_context = loop_run_context("builtin-echo").await;
    let io = Arc::new(ProductLiveCapabilityIo::default());
    let input_ref = io
        .stage_input(
            &run_context,
            serde_json::json!({ "message": "hello product live" }),
        )
        .unwrap();
    let capability_id = capability_id(ECHO_CAPABILITY_ID);
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: authority_resolver(
                ProductLiveVisibleCapabilityRequestConfig::new(
                    UserId::new("user-builtin-echo").unwrap(),
                    RuntimeKind::FirstParty,
                    TrustClass::FirstParty,
                    SurfaceKind::new("agent_loop").unwrap(),
                    CapabilitySurfacePolicy::allow_all(),
                )
                .with_grants(dispatch_grants_for_user(
                    UserId::new("user-builtin-echo").unwrap(),
                    [ECHO_CAPABILITY_ID],
                ))
                .with_provider_trust(
                    ExtensionId::new("builtin").unwrap(),
                    EffectiveTrustClass::user_trusted(),
                ),
            ),
            capability_input_resolver: io.clone(),
            capability_result_writer: io.clone(),
            capability_allow_set: capability_allowlist([capability_id.clone()]),
            ..adapter_config()
        },
    )
    .unwrap();
    let capability_port = adapters
        .capability_factory
        .create_capability_port(&run_context)
        .await
        .unwrap();

    let surface = capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == capability_id),
        "builtin echo must be visible through the product-live adapter surface"
    );

    let outcome = capability_port
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: capability_id.clone(),
            input_ref,
        })
        .await
        .unwrap();
    let CapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed builtin echo outcome, got {outcome:?}");
    };
    assert_eq!(completed.safe_summary, "capability completed");
    assert_eq!(
        io.result_for_ref(&run_context, &completed.result_ref)
            .unwrap(),
        serde_json::json!("hello product live")
    );
}

#[tokio::test]
async fn local_dev_adapter_invokes_builtin_shell_through_product_live_surface() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "builtin-shell-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let run_context = loop_run_context("builtin-shell").await;
    let io = Arc::new(ProductLiveCapabilityIo::default());
    let input_ref = io
        .stage_input(
            &run_context,
            serde_json::json!({ "command": "echo hello product shell" }),
        )
        .unwrap();
    let capability_id = capability_id(SHELL_CAPABILITY_ID);
    let shell_effects = vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::Network,
        EffectKind::SpawnProcess,
        EffectKind::ExecuteCode,
    ];
    let user_id = UserId::new("user-builtin-shell").unwrap();
    let shell_grant = CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id.clone(),
        grantee: Principal::User(user_id.clone()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: shell_effects.clone(),
            mounts: MountView::default(),
            network: NetworkPolicy {
                allowed_targets: vec![NetworkTargetPattern {
                    scheme: None,
                    host_pattern: "*".to_string(),
                    port: None,
                }],
                deny_private_ip_ranges: false,
                max_egress_bytes: None,
            },
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    };
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: authority_resolver(
                ProductLiveVisibleCapabilityRequestConfig::new(
                    user_id,
                    RuntimeKind::FirstParty,
                    TrustClass::FirstParty,
                    SurfaceKind::new("agent_loop").unwrap(),
                    CapabilitySurfacePolicy::allow_all(),
                )
                .with_grants(CapabilitySet {
                    grants: vec![shell_grant],
                })
                .with_provider_trust_for_effects(
                    ExtensionId::new("builtin").unwrap(),
                    EffectiveTrustClass::user_trusted(),
                    shell_effects,
                ),
            ),
            capability_input_resolver: io.clone(),
            capability_result_writer: io.clone(),
            capability_allow_set: capability_allowlist([capability_id.clone()]),
            ..adapter_config()
        },
    )
    .unwrap();
    let capability_port = adapters
        .capability_factory
        .create_capability_port(&run_context)
        .await
        .unwrap();

    let surface = capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == capability_id),
        "builtin shell must be visible through the product-live adapter surface"
    );

    let outcome = capability_port
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: capability_id.clone(),
            input_ref,
        })
        .await
        .unwrap();
    let CapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed builtin shell outcome, got {outcome:?}");
    };
    let result = io
        .result_for_ref(&run_context, &completed.result_ref)
        .unwrap();
    assert_eq!(result["exit_code"], serde_json::json!(0));
    assert!(
        result["output"]
            .as_str()
            .expect("shell output must be text")
            .contains("hello product shell")
    );
}

#[tokio::test]
async fn local_dev_adapter_invokes_extension_scoped_grants_with_loop_driver_principal() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "extension-grant-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let run_context = loop_run_context("extension-grant").await;
    let io = Arc::new(ProductLiveCapabilityIo::default());
    let input_ref = io
        .stage_input(
            &run_context,
            serde_json::json!({ "message": "hello extension grant" }),
        )
        .unwrap();
    let capability_id = capability_id(ECHO_CAPABILITY_ID);
    let extension_principal =
        Principal::Extension(loop_driver_execution_extension_id(&run_context).unwrap());
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: authority_resolver(
                ProductLiveVisibleCapabilityRequestConfig::new(
                    UserId::new("user-extension-grant").unwrap(),
                    RuntimeKind::FirstParty,
                    TrustClass::FirstParty,
                    SurfaceKind::new("agent_loop").unwrap(),
                    CapabilitySurfacePolicy::allow_all(),
                )
                .with_grants(grants_for_principal_with_effects(
                    extension_principal,
                    [ECHO_CAPABILITY_ID],
                    vec![EffectKind::DispatchCapability],
                ))
                .with_provider_trust(
                    ExtensionId::new("builtin").unwrap(),
                    EffectiveTrustClass::user_trusted(),
                ),
            ),
            capability_input_resolver: io.clone(),
            capability_result_writer: io.clone(),
            capability_allow_set: capability_allowlist([capability_id.clone()]),
            ..adapter_config()
        },
    )
    .unwrap();
    let capability_port = adapters
        .capability_factory
        .create_capability_port(&run_context)
        .await
        .unwrap();
    let surface = capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == capability_id),
        "extension-scoped grants should authorize the same principal for visibility and invocation"
    );

    let outcome = capability_port
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id,
            input_ref,
        })
        .await
        .unwrap();
    let CapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed extension-grant echo outcome, got {outcome:?}");
    };
    assert_eq!(
        io.result_for_ref(&run_context, &completed.result_ref)
            .unwrap(),
        serde_json::json!("hello extension grant")
    );
}

#[tokio::test]
async fn local_dev_adapter_registers_provider_tool_calls_as_run_scoped_inputs() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "provider-tool-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let run_context = loop_run_context("provider-tool").await;
    let io = Arc::new(ProductLiveCapabilityIo::default());
    let capability_id = capability_id(ECHO_CAPABILITY_ID);
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: authority_resolver(
                ProductLiveVisibleCapabilityRequestConfig::new(
                    UserId::new("user-provider-tool").unwrap(),
                    RuntimeKind::FirstParty,
                    TrustClass::FirstParty,
                    SurfaceKind::new("agent_loop").unwrap(),
                    CapabilitySurfacePolicy::allow_all(),
                )
                .with_grants(dispatch_grants_for_user(
                    UserId::new("user-provider-tool").unwrap(),
                    [ECHO_CAPABILITY_ID],
                ))
                .with_provider_trust(
                    ExtensionId::new("builtin").unwrap(),
                    EffectiveTrustClass::user_trusted(),
                ),
            ),
            capability_input_resolver: io.clone(),
            capability_result_writer: io.clone(),
            capability_allow_set: capability_allowlist([capability_id.clone()]),
            ..adapter_config()
        },
    )
    .unwrap();
    let capability_port = adapters
        .capability_factory
        .create_capability_port(&run_context)
        .await
        .unwrap();
    capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    let tool_definition = capability_port
        .tool_definitions()
        .unwrap()
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .expect("builtin echo should be advertised as a provider tool");
    assert!(
        tool_definition
            .parameters
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|properties| properties.contains_key("message")),
        "provider tool definitions should receive resolved built-in input schemas"
    );

    let provider_tool_call = ProviderToolCall {
        provider_id: "nearai".to_string(),
        provider_model_id: "qwen3-coder".to_string(),
        turn_id: Some("provider-turn:provider-tool".to_string()),
        id: "call_provider_echo".to_string(),
        name: tool_definition.name,
        arguments: serde_json::json!({ "message": "hello from provider tool" }),
        response_reasoning: Some("model selected echo".to_string()),
        reasoning: None,
        signature: Some("sig-provider-tool".to_string()),
    };
    let candidate = capability_port
        .register_provider_tool_call(provider_tool_call.clone())
        .await
        .unwrap();

    assert_eq!(candidate.capability_id, capability_id);
    assert!(
        candidate
            .input_ref
            .as_str()
            .starts_with("input:provider-tool-"),
        "provider tool inputs should use opaque provider-tool refs: {}",
        candidate.input_ref.as_str()
    );
    let other_run_context = loop_run_context("provider-tool-other-run").await;
    let other_capability_port = adapters
        .capability_factory
        .create_capability_port(&other_run_context)
        .await
        .unwrap();
    other_capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    let other_candidate = other_capability_port
        .register_provider_tool_call(provider_tool_call)
        .await
        .unwrap();
    assert_ne!(
        candidate.input_ref, other_candidate.input_ref,
        "provider tool input refs must remain scoped by loop run even when the ref is opaque"
    );
    assert!(
        candidate.provider_replay.is_some(),
        "provider replay metadata must survive registration"
    );

    let outcome = capability_port
        .invoke_capability(CapabilityInvocation {
            surface_version: candidate.surface_version,
            capability_id,
            input_ref: candidate.input_ref,
        })
        .await
        .unwrap();
    let CapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed provider echo outcome, got {outcome:?}");
    };
    assert_eq!(
        io.result_for_ref(&run_context, &completed.result_ref)
            .unwrap(),
        serde_json::json!("hello from provider tool")
    );
}

#[tokio::test]
async fn local_dev_adapter_exposes_skill_install_provider_tool_schema_requires_string_content() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "provider-skill-install-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let run_context = loop_run_context("provider-skill-install").await;
    let io = Arc::new(ProductLiveCapabilityIo::default());
    let capability_id = capability_id(SKILL_INSTALL_CAPABILITY_ID);
    let user_id = UserId::new("user-provider-skill-install").unwrap();
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: authority_resolver(
                ProductLiveVisibleCapabilityRequestConfig::new(
                    user_id.clone(),
                    RuntimeKind::FirstParty,
                    TrustClass::FirstParty,
                    SurfaceKind::new("agent_loop").unwrap(),
                    CapabilitySurfacePolicy::allow_all(),
                )
                .with_grants(grants_for_principal_with_effects(
                    Principal::User(user_id),
                    [SKILL_INSTALL_CAPABILITY_ID],
                    vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
                ))
                .with_provider_trust_for_effects(
                    ExtensionId::new("builtin").unwrap(),
                    EffectiveTrustClass::user_trusted(),
                    vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
                ),
            ),
            capability_input_resolver: io.clone(),
            capability_result_writer: io,
            capability_allow_set: capability_allowlist([capability_id.clone()]),
            ..adapter_config()
        },
    )
    .unwrap();
    let capability_port = adapters
        .capability_factory
        .create_capability_port(&run_context)
        .await
        .unwrap();
    capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    let tool_definition = capability_port
        .tool_definitions()
        .unwrap()
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .expect("builtin skill_install should be advertised as a provider tool");

    let properties = tool_definition
        .parameters
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("skill_install schema should expose object properties");
    assert_eq!(
        properties
            .get("content")
            .and_then(|schema| schema.get("type")),
        Some(&serde_json::json!("string")),
        "skill_install content input should be advertised as a string"
    );
    assert!(
        tool_definition
            .parameters
            .get("required")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|required| required.iter().any(|field| field == "content")),
        "skill_install content input should be required"
    );
}

#[tokio::test]
async fn adapter_config_can_authorize_non_dispatch_provider_trust_effects() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "read-effect-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let run_context = loop_run_context("read-effect").await;
    let io = Arc::new(ProductLiveCapabilityIo::default());
    let capability_id = capability_id(READ_FILE_CAPABILITY_ID);
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: authority_resolver(
                ProductLiveVisibleCapabilityRequestConfig::new(
                    UserId::new("user-read-effect").unwrap(),
                    RuntimeKind::FirstParty,
                    TrustClass::FirstParty,
                    SurfaceKind::new("agent_loop").unwrap(),
                    CapabilitySurfacePolicy::allow_all(),
                )
                .with_grants(grants_for_principal_with_effects(
                    Principal::User(UserId::new("user-read-effect").unwrap()),
                    [READ_FILE_CAPABILITY_ID],
                    vec![EffectKind::ReadFilesystem],
                ))
                .with_provider_trust_for_effects(
                    ExtensionId::new("builtin").unwrap(),
                    EffectiveTrustClass::user_trusted(),
                    vec![EffectKind::ReadFilesystem],
                ),
            ),
            capability_input_resolver: io.clone(),
            capability_result_writer: io,
            capability_allow_set: capability_allowlist([capability_id.clone()]),
            ..adapter_config()
        },
    )
    .unwrap();
    let capability_port = adapters
        .capability_factory
        .create_capability_port(&run_context)
        .await
        .unwrap();

    let surface = capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == capability_id),
        "read-only first-party capabilities require provider trust with read_filesystem authority"
    );
}

#[tokio::test]
async fn local_dev_adapter_invokes_read_file_with_configured_mounts() {
    let root = tempfile::tempdir().unwrap();
    let storage_root = root.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("workspace")).unwrap();
    std::fs::write(storage_root.join("workspace/readme.md"), "alpha\nbeta\n").unwrap();
    let services =
        build_reborn_services(RebornBuildInput::local_dev("read-file-owner", storage_root))
            .await
            .unwrap();
    let run_context = loop_run_context("read-file").await;
    let io = Arc::new(ProductLiveCapabilityIo::default());
    let input_ref = io
        .stage_input(
            &run_context,
            serde_json::json!({ "path": "/workspace/readme.md", "limit": 1 }),
        )
        .unwrap();
    let capability_id = capability_id(READ_FILE_CAPABILITY_ID);
    let mounts = read_only_workspace_mounts();
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: authority_resolver(
                ProductLiveVisibleCapabilityRequestConfig::new(
                    UserId::new("user-read-file").unwrap(),
                    RuntimeKind::FirstParty,
                    TrustClass::FirstParty,
                    SurfaceKind::new("agent_loop").unwrap(),
                    CapabilitySurfacePolicy::allow_all(),
                )
                .with_mounts(mounts.clone())
                .with_grants(grants_for_principal_with_effects_and_mounts(
                    Principal::User(UserId::new("user-read-file").unwrap()),
                    [READ_FILE_CAPABILITY_ID],
                    vec![EffectKind::ReadFilesystem],
                    mounts,
                ))
                .with_provider_trust_for_effects(
                    ExtensionId::new("builtin").unwrap(),
                    EffectiveTrustClass::user_trusted(),
                    vec![EffectKind::ReadFilesystem],
                ),
            ),
            capability_input_resolver: io.clone(),
            capability_result_writer: io.clone(),
            capability_allow_set: capability_allowlist([capability_id.clone()]),
            ..adapter_config()
        },
    )
    .unwrap();
    let capability_port = adapters
        .capability_factory
        .create_capability_port(&run_context)
        .await
        .unwrap();
    let surface = capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    let outcome = capability_port
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id,
            input_ref,
        })
        .await
        .unwrap();
    let CapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed read_file outcome, got {outcome:?}");
    };
    let output = io
        .result_for_ref(&run_context, &completed.result_ref)
        .unwrap();
    assert_eq!(output["path"], serde_json::json!("/workspace/readme.md"));
    assert_eq!(output["lines_shown"], serde_json::json!(1));
    assert!(
        output["content"]
            .as_str()
            .expect("read_file content should be text")
            .contains("alpha")
    );
}

#[tokio::test]
async fn adapter_bundle_requires_host_runtime_facade() {
    let result = ProductLivePlannedRuntimeAdapters::from_services(
        &RebornServices::disabled(),
        adapter_config(),
    );

    assert!(matches!(
        result,
        Err(ProductLivePlannedRuntimeAdapterError::MissingHostRuntime)
    ));
}

#[tokio::test]
async fn adapter_bundle_maps_authority_resolution_failure_to_host_error() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "authority-failure-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: Arc::new(FailingAuthorityResolver),
            ..adapter_config()
        },
    )
    .unwrap();
    let context = loop_run_context("authority-failure").await;

    let error = match adapters
        .capability_factory
        .create_capability_port(&context)
        .await
    {
        Ok(_) => panic!("authority resolver failures must map to host errors"),
        Err(error) => error,
    };

    assert_eq!(
        error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::InvalidInvocation
    );
    assert!(
        error
            .safe_summary
            .contains("product-live capability execution scope is invalid"),
        "unexpected error summary: {}",
        error.safe_summary
    );
}

#[tokio::test]
async fn adapter_bundle_wires_required_product_live_components() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "adapter-test-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let adapters =
        ProductLivePlannedRuntimeAdapters::from_services(&services, adapter_config()).unwrap();

    let route = adapters
        .model_route_resolver
        .resolve_model_route(ModelSlot::Default)
        .unwrap();
    assert_eq!(route.route().provider_id(), "nearai");
    assert_eq!(route.route().model_id(), "qwen3-coder");
    assert_eq!(route.policy_mode(), ModelSelectionMode::ManagedOnly);

    let context = loop_run_context("adapter-config").await;
    let allow_set = adapters
        .capability_surface_resolver
        .resolve(&context)
        .await
        .unwrap();
    assert!(allow_set.permits(&capability_id("demo.allowed")));
    assert!(!allow_set.permits(&capability_id("demo.denied")));

    let readiness = verify_product_live_cancellation_probe(adapters.cancellation_factory.as_ref())
        .expect("turn-state cancellation factory should expose a live probe");
    assert_eq!(
        readiness,
        ironclaw_loop_support::ProductLiveCancellationReadiness::ExternallyControllable
    );

    let capability_port = adapters
        .capability_factory
        .create_capability_port(&context)
        .await
        .unwrap();
    let visible = capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(
        !visible.version.as_str().is_empty(),
        "host-runtime capability facade should supply a concrete surface version"
    );
}

#[tokio::test]
async fn adapter_bundle_builds_visible_requests_from_each_run_context() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "multi-run-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let adapters =
        ProductLivePlannedRuntimeAdapters::from_services(&services, adapter_config()).unwrap();
    let first_context = loop_run_context("multi-run-first").await;
    let second_context = loop_run_context("multi-run-second").await;

    for run_context in [&first_context, &second_context] {
        let capability_port = adapters
            .capability_factory
            .create_capability_port(run_context)
            .await
            .unwrap();
        let visible = capability_port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .unwrap();
        assert!(
            !visible.version.as_str().is_empty(),
            "run-scoped visible request should be valid for {}",
            run_context.run_id
        );
    }
}

#[tokio::test]
async fn adapter_bundle_resolves_authority_for_each_run_context() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "run-authority-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: Arc::new(RecordingAuthorityResolver {
                calls: Arc::clone(&calls),
            }),
            capability_allow_set: capability_allowlist([capability_id(ECHO_CAPABILITY_ID)]),
            ..adapter_config()
        },
    )
    .unwrap();
    let first_context = loop_run_context("run-authority-first").await;
    let second_context = loop_run_context("run-authority-second").await;

    for run_context in [&first_context, &second_context] {
        let capability_port = adapters
            .capability_factory
            .create_capability_port(run_context)
            .await
            .unwrap();
        let visible = capability_port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .unwrap();
        assert!(
            visible
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id == capability_id(ECHO_CAPABILITY_ID)),
            "per-run authority should authorize echo for {}",
            run_context.run_id
        );
    }

    let recorded = calls.lock().expect("authority call lock poisoned").clone();
    assert_eq!(
        recorded,
        vec![first_context.run_id, second_context.run_id],
        "adapter must resolve capability authority for each run instead of reusing a fixed request"
    );
}

#[tokio::test]
async fn adapter_bundle_satisfies_product_live_runtime_readiness_gate() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "runtime-gate-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let turn_state = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let thread_scope = thread_scope("runtime-gate");
    let adapters =
        ProductLivePlannedRuntimeAdapters::from_services(&services, adapter_config()).unwrap();

    let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_state.clone();
    let loop_checkpoint_for_evidence: Arc<dyn LoopCheckpointStore> = loop_checkpoint_store.clone();
    let composition = build_product_live_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state,
        thread_service: Arc::clone(&thread_service) as Arc<dyn SessionThreadService>,
        thread_scope: thread_scope.clone(),
        model_gateway: Arc::new(StubModelGateway),
        checkpoint_state_store: checkpoint_state_store as Arc<dyn CheckpointStateStore>,
        loop_checkpoint_store,
        milestone_sink,
        capability_factory: adapters.capability_factory,
        capability_surface_resolver: adapters.capability_surface_resolver,
        capability_result_writer: adapters.capability_result_writer,
        subagent_goal_store: Arc::new(InMemoryBoundedSubagentGoalStore::new()),
        subagent_gate_store: Arc::new(BoundedSubagentGateResolutionStore::new()),
        subagent_definition_resolver: Arc::new(StaticSubagentDefinitionResolver),
        subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(
            adapters.capability_input_resolver,
        )),
        subagent_spawn_limits: ironclaw_loop_support::SubagentSpawnLimits::default(),
        loop_exit_evidence: Arc::new(ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            thread_service,
            turn_state_for_evidence,
            loop_checkpoint_for_evidence,
            thread_scope,
        )),
        config: DefaultPlannedRuntimeConfig::default(),
        model_route_resolver: Some(adapters.model_route_resolver),
        cancellation_factory: Some(adapters.cancellation_factory),
        skill_context_source: None,
        input_queue: Some(adapters.input_queue),
        identity_context_source: adapters.identity_context_source,
        model_policy_guard: Some(adapters.model_policy_guard),
        model_budget_accountant: Some(adapters.model_budget_accountant),
        model_response_observer: None,
        safety_context: Some(adapters.safety_context),
        turn_event_sink: None,
    })
    .expect("adapter bundle should satisfy the product-live readiness gate");

    let profile = composition
        .run_profile_resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    assert_eq!(profile.profile_id.as_str(), "reborn-planned-default");
}

#[tokio::test]
async fn model_route_settings_wire_default_and_mission_slots() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "route-settings-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let settings = ProductLiveModelRouteSettings::new("nearai", "qwen3-coder")
        .unwrap()
        .with_mission_route("openrouter", "anthropic/claude-sonnet-4")
        .unwrap();
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            model_routes: settings,
            ..adapter_config()
        },
    )
    .unwrap();

    let mission = adapters
        .model_route_resolver
        .resolve_model_route(ModelSlot::Mission)
        .unwrap();
    assert_eq!(mission.route().provider_id(), "openrouter");
    assert_eq!(mission.route().model_id(), "anthropic/claude-sonnet-4");
}

#[tokio::test]
async fn model_route_settings_respect_selection_mode_override() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "route-selection-mode-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let settings = ProductLiveModelRouteSettings::new("nearai", "qwen3-coder")
        .unwrap()
        .with_selection_mode(ModelSelectionMode::DeveloperAnyConfigured);
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            model_routes: settings,
            ..adapter_config()
        },
    )
    .unwrap();

    let default = adapters
        .model_route_resolver
        .resolve_model_route(ModelSlot::Default)
        .unwrap();
    assert_eq!(
        default.policy_mode(),
        ModelSelectionMode::DeveloperAnyConfigured
    );
}

fn adapter_config() -> ProductLivePlannedRuntimeAdapterConfig {
    ProductLivePlannedRuntimeAdapterConfig {
        capability_authority_resolver: authority_resolver(visible_capability_request_config(
            "adapter-config",
        )),
        capability_input_resolver: Arc::new(UnusedCapabilityIo),
        capability_result_writer: Arc::new(UnusedCapabilityIo),
        capability_allow_set: capability_allowlist([capability_id("demo.allowed")]),
        model_routes: ProductLiveModelRouteSettings::new("nearai", "qwen3-coder").unwrap(),
        cancellation_factory: Arc::new(ReadyRunCancellationFactory::default()),
        input_queue: Arc::new(EmptyInputQueue),
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: Arc::new(NoOpPolicyGuard) as Arc<dyn LoopModelPolicyGuard>,
        model_budget_accountant: Arc::new(NoOpBudgetAccountant)
            as Arc<dyn LoopModelBudgetAccountant>,
        safety_context: InstructionSafetyContext::new(
            "policy:adapter-test",
            "adapter test safety policy",
        )
        .unwrap(),
        milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
    }
}

fn visible_capability_request_config(label: &str) -> ProductLiveVisibleCapabilityRequestConfig {
    ProductLiveVisibleCapabilityRequestConfig::new(
        UserId::new(format!("user-{label}")).unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        SurfaceKind::new("agent_loop").unwrap(),
        CapabilitySurfacePolicy::allow_all(),
    )
}

fn authority_resolver(
    config: ProductLiveVisibleCapabilityRequestConfig,
) -> Arc<dyn ProductLiveCapabilityAuthorityResolver> {
    Arc::new(StaticAuthorityResolver { config })
}

struct StaticAuthorityResolver {
    config: ProductLiveVisibleCapabilityRequestConfig,
}

#[async_trait]
impl ProductLiveCapabilityAuthorityResolver for StaticAuthorityResolver {
    async fn resolve_capability_authority(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<ProductLiveVisibleCapabilityRequestConfig, ProductLivePlannedRuntimeAdapterError>
    {
        Ok(self.config.clone())
    }
}

struct FailingAuthorityResolver;

#[async_trait]
impl ProductLiveCapabilityAuthorityResolver for FailingAuthorityResolver {
    async fn resolve_capability_authority(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<ProductLiveVisibleCapabilityRequestConfig, ProductLivePlannedRuntimeAdapterError>
    {
        Err(
            ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
                reason: "authority unavailable".to_string(),
            },
        )
    }
}

struct RecordingAuthorityResolver {
    calls: Arc<Mutex<Vec<TurnRunId>>>,
}

#[async_trait]
impl ProductLiveCapabilityAuthorityResolver for RecordingAuthorityResolver {
    async fn resolve_capability_authority(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<ProductLiveVisibleCapabilityRequestConfig, ProductLivePlannedRuntimeAdapterError>
    {
        let call_index = {
            let mut calls = self.calls.lock().expect("authority call lock poisoned");
            calls.push(run_context.run_id);
            calls.len()
        };
        let user_id = UserId::new(format!("user-run-authority-{call_index}")).unwrap();
        Ok(ProductLiveVisibleCapabilityRequestConfig::new(
            user_id.clone(),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            SurfaceKind::new("agent_loop").unwrap(),
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_grants(dispatch_grants_for_user(user_id, [ECHO_CAPABILITY_ID]))
        .with_provider_trust(
            ExtensionId::new("builtin").unwrap(),
            EffectiveTrustClass::user_trusted(),
        ))
    }
}

async fn loop_run_context(label: &str) -> LoopRunContext {
    let context = host_visible_capability_request(label).context;
    let resolved = default_planned_run_profile_resolver()
        .unwrap()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    LoopRunContext::new(
        TurnScope::new(
            context.tenant_id,
            context.agent_id,
            context.project_id,
            context.thread_id.unwrap(),
        ),
        TurnId::new(),
        TurnRunId::new(),
        resolved,
    )
}

fn host_visible_capability_request(label: &str) -> HostVisibleCapabilityRequest {
    let mut context = ExecutionContext::local_default(
        UserId::new(format!("user-{label}")).unwrap(),
        ExtensionId::new("adapter-test").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        ironclaw_host_api::MountView::default(),
    )
    .unwrap();
    let thread_id = ThreadId::new(format!("thread-{label}")).unwrap();
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id);
    HostVisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
}

fn thread_scope(label: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new(format!("tenant-{label}")).unwrap(),
        agent_id: AgentId::new(format!("agent-{label}")).unwrap(),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    }
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn dispatch_grants_for_user<const N: usize>(
    user_id: UserId,
    capabilities: [&str; N],
) -> CapabilitySet {
    grants_for_principal_with_effects(
        Principal::User(user_id),
        capabilities,
        vec![EffectKind::DispatchCapability],
    )
}

fn grants_for_principal_with_effects<const N: usize>(
    grantee: Principal,
    capabilities: [&str; N],
    allowed_effects: Vec<EffectKind>,
) -> CapabilitySet {
    grants_for_principal_with_effects_and_mounts(
        grantee,
        capabilities,
        allowed_effects,
        MountView::default(),
    )
}

fn grants_for_principal_with_effects_and_mounts<const N: usize>(
    grantee: Principal,
    capabilities: [&str; N],
    allowed_effects: Vec<EffectKind>,
    mounts: MountView,
) -> CapabilitySet {
    CapabilitySet {
        grants: capabilities
            .into_iter()
            .map(|capability| {
                grant_for_principal_with_effects(
                    grantee.clone(),
                    capability,
                    allowed_effects.clone(),
                    mounts.clone(),
                )
            })
            .collect(),
    }
}

fn grant_for_principal_with_effects(
    grantee: Principal,
    capability: &str,
    allowed_effects: Vec<EffectKind>,
    mounts: MountView,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(capability),
        grantee,
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects,
            mounts,
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn read_only_workspace_mounts() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap()
}

struct EmptyInputQueue;

#[async_trait]
impl HostInputQueue for EmptyInputQueue {
    async fn next_after(
        &self,
        _run_id: TurnRunId,
        after: ironclaw_turns::run_profile::LoopInputCursorToken,
        _limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        Ok(HostInputBatch {
            inputs: Vec::<HostInputEnvelope>::new(),
            next_cursor: after,
        })
    }

    async fn ack_consumed(
        &self,
        _run_id: TurnRunId,
        _tokens: Vec<ironclaw_turns::run_profile::LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        Ok(())
    }
}

struct EmptyIdentityContextSource;

#[async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

struct UnusedCapabilityIo;

#[async_trait]
impl LoopCapabilityInputResolver for UnusedCapabilityIo {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        Ok(serde_json::json!({}))
    }
}

#[async_trait]
impl LoopCapabilityResultWriter for UnusedCapabilityIo {
    async fn write_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
        _invocation_id: InvocationId,
        _capability_id: &CapabilityId,
        _output: serde_json::Value,
    ) -> Result<LoopResultRef, AgentLoopHostError> {
        Ok(LoopResultRef::new("result:adapter-test").unwrap())
    }
}

struct StubModelGateway;

#[async_trait]
impl HostManagedModelGateway for StubModelGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model gateway not exercised by adapter readiness test",
        ))
    }
}

#[derive(Default)]
struct ReadyRunCancellationFactory {
    handles: Arc<Mutex<HashMap<TurnRunId, RunCancellationHandle>>>,
}

#[async_trait]
impl RunCancellationFactory for ReadyRunCancellationFactory {
    async fn handle_for_run(
        &self,
        _scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        let handle = RunCancellationHandle::default();
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .insert(run_id, handle.clone());
        Ok(handle)
    }

    fn product_live_cancellation_probe(&self) -> Option<Box<dyn ProductLiveCancellationProbe>> {
        Some(Box::new(ReadyCancellationProbe {
            handle: RunCancellationHandle::default(),
        }))
    }
}

struct ReadyCancellationProbe {
    handle: RunCancellationHandle,
}

impl ProductLiveCancellationProbe for ReadyCancellationProbe {
    fn request_cancellation(
        &self,
        reason_kind: LoopCancelReasonKind,
    ) -> Result<(), AgentLoopHostError> {
        self.handle.request(reason_kind);
        Ok(())
    }

    fn is_cancellation_observed(&self) -> Result<bool, AgentLoopHostError> {
        Ok(self.handle.is_requested())
    }
}
