use std::{sync::Arc, thread, time::Duration};

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_approvals::LeaseApproval;
use ironclaw_authorization::{
    CapabilityLeaseStatus, CapabilityLeaseStore, GrantAuthorizer, InMemoryCapabilityLeaseStore,
    TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventReplay, EventStreamKey,
    InMemoryAuditSink, InMemoryDurableAuditLog, InMemoryDurableEventLog, InMemoryEventSink,
    ReadScope, RuntimeEventKind,
};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CancelReason, CancelRuntimeWorkRequest, CapabilitySurfaceVersion, DefaultHostRuntime,
    HostRuntime, HostRuntimeServices, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeFailureKind, RuntimeStatusRequest, RuntimeWorkId,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutionResult, McpExecutor};
use ironclaw_processes::{
    InMemoryProcessResultStore, InMemoryProcessStore, ProcessResultStore, ProcessServices,
    ProcessStart, ProcessStatus, ProcessStore,
};
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits,
};
use ironclaw_run_state::{
    InMemoryApprovalRequestStore, InMemoryRunStateStore, RunStateStore, RunStatus,
};
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptRuntime, ScriptRuntimeConfig,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use ironclaw_wasm::{
    RecordingWasmHostHttp, WasmHostError, WasmHostHttp, WasmHttpRequest, WasmHttpResponse,
    WitToolHost, WitToolRuntimeConfig,
};
use serde_json::json;
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_parser::Resolve;

#[tokio::test]
async fn host_runtime_services_builds_dispatcher_runtime_and_health_from_registered_adapters() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(GrantAuthorizer::new());
    let process_services = ProcessServices::in_memory();
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let events = InMemoryEventSink::new();
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));

    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_capability_leases(capability_leases)
    .with_script_runtime(script_runtime)
    .with_event_sink(Arc::new(events.clone()));

    let runtime = services.host_runtime();
    let context = execution_context_with_dispatch_grant(script_capability_id());
    let request = RuntimeCapabilityRequest::new(
        context,
        script_capability_id(),
        ResourceEstimate::default(),
        json!({"message": "from services"}),
        trust_decision_with_dispatch_authority(),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, json!({"message": "from services"}));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let health = runtime.health().await.unwrap();
    assert!(
        health.ready,
        "registered script adapter should make health ready"
    );
    assert!(health.missing_runtime_backends.is_empty());
    let kinds = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
}

#[tokio::test]
async fn host_runtime_services_writes_runtime_events_to_durable_event_log_metadata_only() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(GrantAuthorizer::new());
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_durable_event_log(Arc::clone(&event_log))
    .with_script_runtime(script_runtime);
    let scope = sample_scope(InvocationId::new());
    let payload = json!({
        "message": "RAW_EVENT_INPUT_SENTINEL_3147 /tmp/private-event-path",
        "secret": "SECRET_EVENT_SENTINEL_3147_sk_live_secret",
        "output": "RUNTIME_EVENT_OUTPUT_SENTINEL_3147",
    });

    let outcome = services
        .host_runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone()),
            script_capability_id(),
            ResourceEstimate::default(),
            payload.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.output, payload);
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }

    let replay = event_log
        .read_after_cursor(
            &EventStreamKey::from_scope(&scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .unwrap();
    let kinds = replay
        .entries
        .iter()
        .map(|entry| entry.record.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
    assert_eq!(
        replay.entries[2].record.output_bytes,
        Some(serde_json::to_vec(&payload).unwrap().len() as u64)
    );

    let serialized = serde_json::to_string(&replay).unwrap();
    for forbidden in [
        "RAW_EVENT_INPUT_SENTINEL_3147",
        "/tmp/private-event-path",
        "SECRET_EVENT_SENTINEL_3147",
        "RUNTIME_EVENT_OUTPUT_SENTINEL_3147",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "durable runtime event replay leaked {forbidden}: {serialized}"
        );
    }
    assert!(serialized.contains("script.echo"));
    assert!(serialized.contains("dispatch_requested"));
    assert!(serialized.contains("dispatch_succeeded"));
}

#[tokio::test]
async fn host_runtime_services_resumes_approved_capability_and_consumes_lease_once() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approval resume"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;

    let resumed = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match resumed {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, input);
        }
        other => panic!("expected completed resume outcome, got {other:?}"),
    }
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Consumed
    );
    let kinds = fixture
        .events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );

    let second = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(second, RuntimeFailureKind::Authorization);
    assert_eq!(
        fixture.events.events().len(),
        3,
        "second resume must fail before a second dispatch"
    );
}

#[tokio::test]
async fn host_runtime_services_resume_changed_input_fails_before_lease_claim_or_dispatch() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let original_input = json!({"message": "original"});

    let gate =
        block_for_approval(&runtime, context.clone(), estimate.clone(), original_input).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;

    let outcome = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            json!({"message": "changed"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Authorization);
    assert!(fixture.events.events().is_empty());
    // The approval request stores the original invocation fingerprint; changed input
    // computes a different resume fingerprint, so no matching lease is claimable.
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active,
        "fingerprint mismatch must fail before lease claim/consume"
    );
}

#[tokio::test]
async fn host_runtime_services_resume_wrong_user_scope_is_hidden_before_dispatch() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "wrong user"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;
    let wrong_scope = ResourceScope {
        user_id: UserId::new("other-user").unwrap(),
        ..scope.clone()
    };
    let wrong_context =
        execution_context_with_dispatch_grant_for_scope(script_capability_id(), wrong_scope);

    let outcome = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            wrong_context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert!(fixture.events.events().is_empty());
    let original_run = fixture
        .run_state
        .get(&scope, context.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original_run.status, RunStatus::BlockedApproval);
    assert_eq!(
        original_run.approval_request_id,
        Some(gate.approval_request_id)
    );
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn host_runtime_services_resume_expired_lease_fails_before_dispatch() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "expired"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease = approve_dispatch_for_services(
        &fixture.services,
        &scope,
        gate.approval_request_id,
        Some(Utc::now() - ChronoDuration::seconds(1)),
    )
    .await;

    let outcome = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Authorization);
    assert!(fixture.events.events().is_empty());
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn host_runtime_services_resume_trust_preflight_failure_fails_only_matching_blocked_run() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "stale trust metadata"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;
    let broken_runtime = resume_runtime_with_empty_registry(&fixture);

    let wrong_scope = ResourceScope {
        user_id: UserId::new("other-user").unwrap(),
        ..scope.clone()
    };
    let wrong_context = execution_context_without_grants_for_scope(wrong_scope);
    let wrong_scope_outcome = broken_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            wrong_context,
            gate.approval_request_id,
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert_failed_outcome(wrong_scope_outcome, RuntimeFailureKind::MissingRuntime);
    assert_blocked_approval_run(
        &fixture,
        &scope,
        context.invocation_id,
        gate.approval_request_id,
    )
    .await;

    let mut invalid_context = context.clone();
    invalid_context.user_id = UserId::new("tampered-user").unwrap();
    let invalid_context_outcome = broken_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            invalid_context,
            gate.approval_request_id,
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert_failed_outcome(invalid_context_outcome, RuntimeFailureKind::MissingRuntime);
    assert_blocked_approval_run(
        &fixture,
        &scope,
        context.invocation_id,
        gate.approval_request_id,
    )
    .await;

    let matching_outcome = broken_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert_failed_outcome(matching_outcome, RuntimeFailureKind::MissingRuntime);

    let failed_run = fixture
        .run_state
        .get(&scope, context.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed_run.status, RunStatus::Failed);
    assert_eq!(failed_run.approval_request_id, None);
    assert_eq!(failed_run.error_kind.as_deref(), Some("unknown_capability"));
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active,
        "trust preflight failure must not claim or consume the approval lease"
    );
    assert!(fixture.events.events().is_empty());
}

#[tokio::test]
async fn host_runtime_services_resume_without_backing_stores_fails_closed() {
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .host_runtime();

    let outcome = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            execution_context_without_grants(),
            ApprovalRequestId::new(),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "missing stores"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn host_runtime_services_registered_runtime_health_tracks_script_mcp_and_wasm_adapters() {
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifests(&[
            SCRIPT_MANIFEST,
            MCP_MANIFEST,
            WASM_MANIFEST,
        ])),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_script_runtime(script_runtime)
    .with_mcp_runtime(Arc::new(PanicMcpExecutor))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap()
    .host_runtime();

    let health = runtime.health().await.unwrap();

    assert!(health.ready);
    assert!(health.missing_runtime_backends.is_empty());
}

#[tokio::test]
async fn host_runtime_services_health_fails_closed_for_unregistered_required_runtime() {
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .host_runtime();

    let health = runtime.health().await.unwrap();

    assert!(!health.ready);
    assert_eq!(health.missing_runtime_backends, vec![RuntimeKind::Script]);
}

#[tokio::test]
async fn host_runtime_services_installs_builtin_obligation_handler_with_audit_sink() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let audit = Arc::new(InMemoryAuditSink::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![Obligation::AuditBefore]));
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_audit_sink(Arc::clone(&audit))
    .with_script_runtime(script_runtime);

    let outcome = services
        .host_runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant(script_capability_id()),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "audited through services"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(
                completed.output,
                json!({"message": "audited through services"})
            );
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].stage, AuditStage::Before);
    assert_eq!(records[0].action.target.as_deref(), Some("script.echo"));
}

#[tokio::test]
async fn host_runtime_services_writes_obligation_audit_records_to_durable_log_metadata_only() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::AuditBefore,
            Obligation::AuditAfter,
        ]));
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_durable_audit_log(Arc::clone(&audit_log))
    .with_script_runtime(script_runtime);
    let scope = sample_scope(InvocationId::new());
    let payload = json!({
        "message": "RAW_INPUT_SENTINEL_3147 /tmp/private-host-path",
        "secret": "SECRET_SENTINEL_3147_sk_live_secret",
        "output": "RUNTIME_OUTPUT_SENTINEL_3147",
    });

    let outcome = services
        .host_runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone()),
            script_capability_id(),
            ResourceEstimate::default(),
            payload.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.output, payload);
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let replay = audit_log
        .read_after_cursor(
            &EventStreamKey::from_scope(&scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(replay.entries[0].record.stage, AuditStage::Before);
    assert_eq!(replay.entries[1].record.stage, AuditStage::After);
    assert_eq!(
        replay.entries[1]
            .record
            .result
            .as_ref()
            .and_then(|result| result.output_bytes),
        Some(serde_json::to_vec(&payload).unwrap().len() as u64)
    );

    let serialized = serde_json::to_string(&replay).unwrap();
    for forbidden in [
        "RAW_INPUT_SENTINEL_3147",
        "/tmp/private-host-path",
        "SECRET_SENTINEL_3147",
        "RUNTIME_OUTPUT_SENTINEL_3147",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "durable obligation audit replay leaked {forbidden}: {serialized}"
        );
    }
    assert!(serialized.contains("script.echo"));
    assert!(serialized.contains("audit_before"));
    assert!(serialized.contains("audit_after"));
}

#[tokio::test]
async fn host_runtime_services_fails_closed_when_durable_obligation_audit_append_fails() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![Obligation::AuditBefore]));
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_durable_audit_log(Arc::new(FailingDurableAuditLog))
    .with_script_runtime(script_runtime);

    let outcome = services
        .host_runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant(script_capability_id()),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "must not dispatch after audit append failure"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind, RuntimeFailureKind::Backend);
            let message = failure.message.unwrap_or_default();
            assert!(message.contains("obligation handling failed: Audit"));
            assert!(
                !message.contains("/tmp/audit-backend-secret"),
                "audit backend details must remain sanitized: {message}"
            );
        }
        other => panic!("expected failed outcome, got {other:?}"),
    }
}

#[tokio::test]
async fn host_runtime_services_routes_wasm_http_through_per_invocation_policy_handoff() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
        ]));
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::clone(&egress))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();
    let scope = sample_scope(InvocationId::new());

    let outcome = services
        .host_runtime()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope.clone(),
            json!({"call": "http-success"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.output, json!(1));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].runtime, RuntimeKind::Wasm);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].network_policy, policy);
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert_eq!(requests[0].url, "https://example.test/api");
    assert_eq!(requests[0].body, b"hello".to_vec());
}

#[tokio::test]
async fn host_runtime_services_routes_cached_wasm_http_through_per_invocation_policy_handoff() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
        ]));
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::clone(&egress))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap();
    let runtime = services.host_runtime();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();
    let first_scope = sample_scope(InvocationId::new());
    let second_scope = sample_scope(InvocationId::new());

    let first = runtime
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            first_scope.clone(),
            json!({"call": "http-success-first"}),
        ))
        .await
        .unwrap();
    let second = runtime
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            second_scope.clone(),
            json!({"call": "http-success-second"}),
        ))
        .await
        .unwrap();

    assert_completed_outcome(first, &capability_id);
    assert_completed_outcome(second, &capability_id);
    let requests = egress.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].scope, first_scope);
    assert_eq!(requests[1].scope, second_scope);
    assert_eq!(requests[0].network_policy, policy);
    assert_eq!(requests[1].network_policy, policy);
}

#[tokio::test]
async fn host_runtime_services_denies_wasm_http_when_shared_egress_has_no_policy_handoff() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let direct_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: Vec::new(),
    }));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        Arc::new(AllowAllDispatchAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::clone(&egress))
    .try_with_wasm_runtime(
        WitToolRuntimeConfig::for_testing(),
        WitToolHost::deny_all().with_http(Arc::clone(&direct_http)),
    )
    .unwrap();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();

    let outcome = services
        .host_runtime()
        .invoke_capability(wasm_runtime_request(
            capability_id,
            json!({"call": "http-without-policy"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.usage.network_egress_bytes, 0);
        }
        RuntimeCapabilityOutcome::Failed(_) => {}
        other => panic!("expected completed or failed outcome, got {other:?}"),
    }
    assert!(egress.requests().is_empty());
    assert!(
        direct_http.requests().unwrap().is_empty(),
        "HostRuntimeServices must not let a preconfigured WASM host bypass policy handoff when shared egress is active"
    );
}

#[test]
fn host_runtime_services_wasm_input_encode_releases_prepared_reservation() {
    let services = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services.rs"),
    )
    .unwrap();
    let reservation_index = services
        .find("let reservation = match request.resource_reservation")
        .expect("WASM execution must bind the dispatch reservation");
    let input_index = services
        .find("let input_json = match serde_json::to_string(&request.input)")
        .expect("WASM input encoding must use explicit cleanup branch");

    assert!(
        reservation_index < input_index,
        "WASM adapters must take ownership of a prepared reservation before input encoding so encode failures can release it"
    );
    assert!(
        services.contains(
            "Err(_) => {\n            release_wasm_reservation(request.governor, reservation.id);"
        ),
        "InputEncode failures must release the prepared WASM reservation"
    );
}

#[tokio::test]
async fn host_runtime_services_cancel_and_status_share_process_result_and_cancellation_graph() {
    let process_services = ProcessServices::in_memory();
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let cancellation_registry = process_services.cancellation_registry();
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let runtime = HostRuntimeServices::new(
        registry,
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .host_runtime();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id);
    let token = cancellation_registry.register(&scope, process_id);
    process_store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let status = runtime
        .runtime_status(RuntimeStatusRequest::new(
            scope.clone(),
            CorrelationId::new(),
        ))
        .await
        .unwrap();
    assert_eq!(status.active_work.len(), 1);
    assert_eq!(
        status.active_work[0].work_id,
        RuntimeWorkId::Process(process_id)
    );

    let outcome = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            scope.clone(),
            CorrelationId::new(),
            CancelReason::UserRequested,
        ))
        .await
        .unwrap();

    assert_eq!(outcome.cancelled, vec![RuntimeWorkId::Process(process_id)]);
    assert!(token.is_cancelled());
    let result = result_store.get(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(result.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn host_runtime_services_wasm_guest_error_reconciles_usage_after_host_effect() {
    let wat = http_then_guest_error_wat();
    let runtime = wasm_runtime_for_component(
        WASM_GUEST_ERROR_MANIFEST,
        "wasm-accounting.guest_error",
        "wasm/guest-error.wasm",
        &wat,
    )
    .await;

    let outcome = runtime
        .runtime
        .invoke_capability(wasm_runtime_request(
            runtime.capability_id,
            json!({"call": "guest-error"}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert_eq!(runtime.http.requests().unwrap().len(), 1);
    assert_eq!(
        runtime
            .governor
            .usage_for(&sample_account())
            .network_egress_bytes,
        5,
        "host-mediated HTTP request bytes must be reconciled even when the guest returns an error response"
    );
    assert_eq!(
        runtime
            .governor
            .reserved_for(&sample_account())
            .network_egress_bytes,
        0
    );
}

#[tokio::test]
async fn host_runtime_services_wasm_invalid_output_reconciles_usage_after_host_effect() {
    let wat = http_then_invalid_output_wat();
    let runtime = wasm_runtime_for_component(
        WASM_INVALID_OUTPUT_MANIFEST,
        "wasm-accounting.invalid_output",
        "wasm/invalid-output.wasm",
        &wat,
    )
    .await;

    let outcome = runtime
        .runtime
        .invoke_capability(wasm_runtime_request(
            runtime.capability_id,
            json!({"call": "invalid-output"}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::InvalidInput);
    assert_eq!(runtime.http.requests().unwrap().len(), 1);
    assert_eq!(
        runtime
            .governor
            .usage_for(&sample_account())
            .network_egress_bytes,
        5,
        "host-mediated HTTP request bytes must be reconciled even when the guest returns malformed output"
    );
    assert_eq!(
        runtime
            .governor
            .reserved_for(&sample_account())
            .network_egress_bytes,
        0
    );
}

#[tokio::test]
async fn host_runtime_services_wasm_guest_error_reconciles_wall_clock_after_host_effect() {
    let wat = http_without_body_then_guest_error_wat();
    let runtime = wasm_runtime_for_component_with_slow_zero_body_http(
        WASM_WALL_CLOCK_FAILURE_MANIFEST,
        "wasm-accounting.wall_clock_failure",
        "wasm/wall-clock-failure.wasm",
        &wat,
    )
    .await;

    let outcome = runtime
        .runtime
        .invoke_capability(wasm_runtime_request(
            runtime.capability_id,
            json!({"call": "wall-clock-failure"}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert_eq!(runtime.http.requests().unwrap().len(), 1);
    let usage = runtime.governor.usage_for(&sample_account());
    assert!(
        usage.wall_clock_ms > 0,
        "wall-clock usage must be reconciled even when a failed guest has no byte/token/process usage"
    );
    assert_eq!(usage.network_egress_bytes, 0);
    assert_eq!(
        runtime
            .governor
            .reserved_for(&sample_account())
            .network_egress_bytes,
        0
    );
}

fn assert_failed_outcome(outcome: RuntimeCapabilityOutcome, expected_kind: RuntimeFailureKind) {
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => assert_eq!(failure.kind, expected_kind),
        other => panic!("expected failed outcome, got {other:?}"),
    }
}

fn assert_completed_outcome(outcome: RuntimeCapabilityOutcome, expected_capability: &CapabilityId) {
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(&completed.capability_id, expected_capability);
            assert_eq!(completed.output, json!(1));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
}

type InMemoryHostRuntimeServices = HostRuntimeServices<
    LocalFilesystem,
    InMemoryResourceGovernor,
    InMemoryProcessStore,
    InMemoryProcessResultStore,
>;

struct ApprovalResumeFixture {
    services: InMemoryHostRuntimeServices,
    run_state: Arc<InMemoryRunStateStore>,
    approval_requests: Arc<InMemoryApprovalRequestStore>,
    capability_leases: Arc<InMemoryCapabilityLeaseStore>,
    events: InMemoryEventSink,
}

fn approval_resume_fixture() -> ApprovalResumeFixture {
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let events = InMemoryEventSink::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .with_event_sink(Arc::new(events.clone()));

    ApprovalResumeFixture {
        services,
        run_state,
        approval_requests,
        capability_leases,
        events,
    }
}

fn resume_runtime_with_empty_registry(fixture: &ApprovalResumeFixture) -> DefaultHostRuntime {
    HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(Arc::clone(&fixture.run_state))
    .with_approval_requests(Arc::clone(&fixture.approval_requests))
    .with_capability_leases(Arc::clone(&fixture.capability_leases))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .host_runtime()
}

async fn assert_blocked_approval_run(
    fixture: &ApprovalResumeFixture,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    approval_request_id: ApprovalRequestId,
) {
    let run = fixture
        .run_state
        .get(scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::BlockedApproval);
    assert_eq!(run.approval_request_id, Some(approval_request_id));
    assert_eq!(run.error_kind, None);
}

async fn block_for_approval(
    runtime: &impl HostRuntime,
    context: ExecutionContext,
    estimate: ResourceEstimate,
    input: serde_json::Value,
) -> ironclaw_host_runtime::RuntimeApprovalGate {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => panic!("expected approval gate, got {other:?}"),
    }
}

async fn approve_dispatch_for_services(
    services: &InMemoryHostRuntimeServices,
    scope: &ResourceScope,
    approval_request_id: ApprovalRequestId,
    expires_at: Option<Timestamp>,
) -> ironclaw_authorization::CapabilityLease {
    services
        .approval_resolver()
        .expect("approval resolver should be configured")
        .approve_dispatch(
            scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap()
}

struct ApprovalThenGrantAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ApprovalThenGrantAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        if context.grants.grants.is_empty() {
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
        } else {
            GrantAuthorizer::new()
                .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
                .await
        }
    }
}

struct EchoScriptBackend;

impl ScriptBackend for EchoScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        let value = serde_json::from_str(&request.stdin_json).map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput::json(value))
    }
}

struct FailingDurableAuditLog;

#[async_trait]
impl DurableAuditLog for FailingDurableAuditLog {
    async fn append(
        &self,
        _record: AuditEnvelope,
    ) -> Result<ironclaw_events::EventLogEntry<AuditEnvelope>, EventError> {
        Err(EventError::DurableLog {
            reason: "simulated audit backend failure at /tmp/audit-backend-secret".to_string(),
        })
    }

    async fn read_after_cursor(
        &self,
        _stream: &EventStreamKey,
        _filter: &ReadScope,
        _after: Option<EventCursor>,
        _limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError> {
        Err(EventError::DurableLog {
            reason: "simulated audit replay failure".to_string(),
        })
    }
}

struct AllowAllDispatchAuthorizer;

struct ObligatingAuthorizer {
    obligations: Vec<Obligation>,
}

impl ObligatingAuthorizer {
    fn new(obligations: Vec<Obligation>) -> Self {
        Self { obligations }
    }
}

#[derive(Debug, Clone, Default)]
struct RecordingRuntimeHttpEgress {
    requests: Arc<std::sync::Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeHttpEgress {
    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
            redaction_applied: false,
        })
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for AllowAllDispatchAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }
}

struct PanicMcpExecutor;

#[async_trait]
impl McpExecutor for PanicMcpExecutor {
    async fn execute_extension_json(
        &self,
        _governor: &dyn ResourceGovernor,
        _request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        panic!("health-only test must not execute MCP runtime")
    }
}

fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    registry_with_manifests(&[manifest])
}

fn registry_with_manifests(manifests: &[&str]) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    for manifest in manifests {
        let manifest = ExtensionManifest::parse(manifest).unwrap();
        let root =
            VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
        let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
        registry.insert(package).unwrap();
    }
    registry
}

fn execution_context_without_grants() -> ExecutionContext {
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Script,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap()
}

fn execution_context_without_grants_for_scope(scope: ResourceScope) -> ExecutionContext {
    let context = ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Script,
        trust: TrustClass::UserTrusted,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context.validate().unwrap();
    context
}

fn execution_context_with_dispatch_grant(capability: CapabilityId) -> ExecutionContext {
    let grants = capability_grants(capability);
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

fn execution_context_with_dispatch_grant_for_scope(
    capability: CapabilityId,
    scope: ResourceScope,
) -> ExecutionContext {
    let context = ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::UserTrusted,
        grants: capability_grants(capability),
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context.validate().unwrap();
    context
}

fn capability_grants(capability: CapabilityId) -> CapabilitySet {
    let mut grants = CapabilitySet::default();
    grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability,
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    grants
}

fn local_manifest_trust_policy(
    extension_id: &str,
    allowed_effects: Vec<EffectKind>,
) -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new(extension_id).unwrap(),
            format!("/system/extensions/{extension_id}/manifest.toml"),
            None,
            HostTrustAssignment::user_trusted(),
            allowed_effects,
            None,
        ),
    ]))])
    .unwrap()
}

fn trust_decision_with_dispatch_authority() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn sample_scope(invocation_id: InvocationId) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id,
    }
}

fn process_start(
    process_id: ProcessId,
    invocation_id: InvocationId,
    scope: ResourceScope,
) -> ProcessStart {
    ProcessStart {
        process_id,
        parent_process_id: None,
        invocation_id,
        scope,
        extension_id: script_extension_id(),
        capability_id: script_capability_id(),
        runtime: RuntimeKind::Script,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        input: json!({"message": "running"}),
    }
}

fn script_extension_id() -> ExtensionId {
    ExtensionId::new("script").unwrap()
}

fn script_capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

struct WasmRuntimeFixture {
    runtime: DefaultHostRuntime,
    governor: Arc<InMemoryResourceGovernor>,
    http: Arc<RecordingWasmHostHttp>,
    capability_id: CapabilityId,
}

struct WasmWallClockRuntimeFixture {
    runtime: DefaultHostRuntime,
    governor: Arc<InMemoryResourceGovernor>,
    http: Arc<SlowZeroBodyWasmHostHttp>,
    capability_id: CapabilityId,
}

#[derive(Debug)]
struct SlowZeroBodyWasmHostHttp {
    requests: std::sync::Mutex<Vec<WasmHttpRequest>>,
    delay: Duration,
}

impl SlowZeroBodyWasmHostHttp {
    fn new(delay: Duration) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            delay,
        }
    }

    fn requests(&self) -> Result<Vec<WasmHttpRequest>, WasmHostError> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| WasmHostError::Failed("slow HTTP request log is poisoned".into()))
    }
}

impl WasmHostHttp for SlowZeroBodyWasmHostHttp {
    fn request(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError> {
        self.requests
            .lock()
            .map_err(|_| WasmHostError::Failed("slow HTTP request log is poisoned".into()))?
            .push(request);
        thread::sleep(self.delay);
        Ok(WasmHttpResponse {
            status: 204,
            headers_json: "{}".to_string(),
            body: Vec::new(),
        })
    }
}

async fn wasm_runtime_for_component(
    manifest: &str,
    capability: &str,
    module_path: &str,
    wat: &str,
) -> WasmRuntimeFixture {
    let parsed_manifest = ExtensionManifest::parse(manifest).unwrap();
    let component = tool_component(wat);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(parsed_manifest.id.as_str(), module_path, &component).await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(AllowAllDispatchAuthorizer);
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: Vec::new(),
    }));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(manifest)),
        filesystem,
        Arc::clone(&governor),
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .try_with_wasm_runtime(
        WitToolRuntimeConfig::for_testing(),
        WitToolHost::deny_all().with_http(Arc::clone(&http)),
    )
    .unwrap();

    WasmRuntimeFixture {
        runtime: services.host_runtime(),
        governor,
        http,
        capability_id: CapabilityId::new(capability).unwrap(),
    }
}

async fn wasm_runtime_for_component_with_slow_zero_body_http(
    manifest: &str,
    capability: &str,
    module_path: &str,
    wat: &str,
) -> WasmWallClockRuntimeFixture {
    let parsed_manifest = ExtensionManifest::parse(manifest).unwrap();
    let component = tool_component(wat);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(parsed_manifest.id.as_str(), module_path, &component).await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(AllowAllDispatchAuthorizer);
    let http = Arc::new(SlowZeroBodyWasmHostHttp::new(Duration::from_millis(25)));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(manifest)),
        filesystem,
        Arc::clone(&governor),
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .try_with_wasm_runtime(
        WitToolRuntimeConfig::for_testing(),
        WitToolHost::deny_all().with_http(Arc::clone(&http)),
    )
    .unwrap();

    WasmWallClockRuntimeFixture {
        runtime: services.host_runtime(),
        governor,
        http,
        capability_id: CapabilityId::new(capability).unwrap(),
    }
}

async fn filesystem_with_wasm_component(
    extension_id: &str,
    module_path: &str,
    wasm_bytes: &[u8],
) -> LocalFilesystem {
    let fs = mounted_empty_extension_root();
    let path =
        VirtualPath::new(format!("/system/extensions/{extension_id}/{module_path}")).unwrap();
    fs.write_file(&path, wasm_bytes).await.unwrap();
    fs
}

fn mounted_empty_extension_root() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor.set_limit(
        account,
        ResourceLimits {
            max_concurrency_slots: Some(10),
            max_network_egress_bytes: Some(10_000),
            max_output_bytes: Some(100_000),
            ..ResourceLimits::default()
        },
    );
    governor
}

fn wasm_runtime_request(
    capability_id: CapabilityId,
    input: serde_json::Value,
) -> RuntimeCapabilityRequest {
    let scope = sample_scope(InvocationId::new());
    wasm_runtime_request_for_scope(capability_id, scope, input)
}

fn wasm_runtime_request_for_scope(
    capability_id: CapabilityId,
    scope: ResourceScope,
    input: serde_json::Value,
) -> RuntimeCapabilityRequest {
    let context = execution_context_with_dispatch_grant_for_scope(capability_id.clone(), scope);
    RuntimeCapabilityRequest::new(
        context,
        capability_id,
        wasm_http_estimate(),
        input,
        trust_decision_with_dispatch_authority(),
    )
}

fn wasm_http_estimate() -> ResourceEstimate {
    ResourceEstimate {
        concurrency_slots: Some(1),
        network_egress_bytes: Some(10),
        output_bytes: Some(10_000),
        ..ResourceEstimate::default()
    }
}

fn sample_account() -> ResourceAccount {
    ResourceAccount::tenant(TenantId::new("tenant-a").unwrap())
}

fn wasm_http_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

fn tool_component(wat_src: &str) -> Vec<u8> {
    let mut module = wat::parse_str(wat_src).unwrap();
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str("tool.wit", include_str!("../../../wit/tool.wit"))
        .unwrap();
    let world = resolve
        .select_world(&[package], Some("sandboxed-tool"))
        .unwrap();

    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();

    let mut encoder = ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true);
    encoder.encode().unwrap()
}

fn http_then_guest_error_wat() -> String {
    HTTP_TOOL_WAT.replace(
        "i32.const 48\n    i32.const 1\n    i32.store\n    i32.const 52\n    i32.const 3072\n    i32.store\n    i32.const 56\n    i32.const 1\n    i32.store\n    i32.const 60\n    i32.const 0\n    i32.store\n    i32.const 48",
        "i32.const 48\n    i32.const 0\n    i32.store\n    i32.const 52\n    i32.const 0\n    i32.store\n    i32.const 56\n    i32.const 0\n    i32.store\n    i32.const 60\n    i32.const 1\n    i32.store\n    i32.const 64\n    i32.const 3072\n    i32.store\n    i32.const 68\n    i32.const 11\n    i32.store\n    i32.const 48",
    )
}

fn http_then_invalid_output_wat() -> String {
    HTTP_TOOL_WAT
        .replace(
            r#"(data (i32.const 3072) "1")"#,
            r#"(data (i32.const 3072) "not-json")"#,
        )
        .replace(
            "i32.const 56\n    i32.const 1\n    i32.store",
            "i32.const 56\n    i32.const 8\n    i32.store",
        )
}

fn http_without_body_then_guest_error_wat() -> String {
    http_then_guest_error_wat().replace(
        "i32.const 1\n    i32.const 256\n    i32.const 5",
        "i32.const 0\n    i32.const 0\n    i32.const 0",
    )
}

const SCRIPT_MANIFEST: &str = r#"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script integration extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"
args = []

[[capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const MCP_MANIFEST: &str = r#"
id = "mcp"
name = "MCP Search"
version = "0.1.0"
description = "MCP integration extension"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/rpc"

[[capabilities]]
id = "mcp.search"
description = "Search through MCP"
effects = ["dispatch_capability", "network"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const WASM_MANIFEST: &str = r#"
id = "wasm"
name = "WASM Count"
version = "0.1.0"
description = "WASM integration extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "tool.wasm"

[[capabilities]]
id = "wasm.count"
description = "Count through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const WASM_HTTP_SUCCESS_MANIFEST: &str = r#"
id = "wasm-http"
name = "WASM HTTP Success"
version = "0.1.0"
description = "WASM HTTP success extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/http-success.wasm"

[[capabilities]]
id = "wasm-http.success"
description = "Call host HTTP then return success"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const WASM_GUEST_ERROR_MANIFEST: &str = r#"
id = "wasm-accounting"
name = "WASM Accounting Guest Error"
version = "0.1.0"
description = "WASM accounting extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/guest-error.wasm"

[[capabilities]]
id = "wasm-accounting.guest_error"
description = "Call host HTTP then return guest error"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const WASM_INVALID_OUTPUT_MANIFEST: &str = r#"
id = "wasm-accounting"
name = "WASM Accounting Invalid Output"
version = "0.1.0"
description = "WASM accounting extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/invalid-output.wasm"

[[capabilities]]
id = "wasm-accounting.invalid_output"
description = "Call host HTTP then return invalid output"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const WASM_WALL_CLOCK_FAILURE_MANIFEST: &str = r#"
id = "wasm-accounting"
name = "WASM Accounting Wall Clock Failure"
version = "0.1.0"
description = "WASM accounting extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/wall-clock-failure.wasm"

[[capabilities]]
id = "wasm-accounting.wall_clock_failure"
description = "Spend wall-clock time through host HTTP then return a guest error"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const HTTP_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (import "near:agent/host@0.3.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.3.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.3.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.3.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.3.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.3.0" "secret-exists" (func $secret_exists (type 4)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 4096))
  (data (i32.const 128) "POST")
  (data (i32.const 160) "https://example.test/api")
  (data (i32.const 224) "{}")
  (data (i32.const 256) "hello")
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "fixture description")
  (data (i32.const 3072) "1")
  (func $schema (result i32)
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 17
    i32.store
    i32.const 16)
  (func $description (result i32)
    i32.const 32
    i32.const 2048
    i32.store
    i32.const 36
    i32.const 19
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    i32.const 128
    i32.const 4
    i32.const 160
    i32.const 24
    i32.const 224
    i32.const 2
    i32.const 1
    i32.const 256
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 512
    call $http_request

    i32.const 48
    i32.const 1
    i32.store
    i32.const 52
    i32.const 3072
    i32.store
    i32.const 56
    i32.const 1
    i32.store
    i32.const 60
    i32.const 0
    i32.store
    i32.const 48)
  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    (local $ret i32)
    global.get $heap
    local.set $ret
    global.get $heap
    local.get $new_size
    i32.add
    global.set $heap
    local.get $ret)
  (func $_initialize)
  (export "near:agent/tool@0.3.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.3.0#execute" (func $post))
  (export "near:agent/tool@0.3.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.3.0#schema" (func $post))
  (export "near:agent/tool@0.3.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.3.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;
