use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::{ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, PackageId, PermissionMode, Principal, ResourceEstimate,
    RuntimeKind, TrustClass,
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices,
    ReportWorkflowStageResultInput, RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind, SurfaceKind,
    WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID, WorkflowStageResultAck, WorkflowStageResultSink,
    WorkflowStageResultSinkError, builtin_first_party_handlers,
    builtin_first_party_handlers_with_workflow_stage_result_sink, builtin_first_party_package,
    builtin_first_party_package_with_workflow_stage_result,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_triggers::InMemoryTriggerRepository;
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::{Value, json};

#[tokio::test]
async fn workflow_result_manifest_is_host_bundled_and_schema_resolves() {
    let package = builtin_first_party_package_with_workflow_stage_result().unwrap();
    assert_eq!(package.manifest.source, ManifestSource::HostBundled);

    let runtime = runtime_with_workflow_sink(Arc::new(RecordingWorkflowSink::default()));
    let surface = runtime
        .visible_capabilities(
            ironclaw_host_runtime::VisibleCapabilityRequest::new(
                execution_context([WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID]),
                SurfaceKind::new("agent_loop").unwrap(),
            )
            .with_policy(CapabilitySurfacePolicy::allow_all())
            .with_provider_trust(provider_trust()),
        )
        .await
        .unwrap();

    let capability = surface
        .capabilities
        .iter()
        .find(|candidate| {
            candidate.descriptor.id.as_str() == WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID
        })
        .expect("workflow result capability is visible");
    assert_eq!(
        capability.descriptor.parameters_schema["required"],
        json!([
            "workflow_run_id",
            "stage_run_id",
            "turn_run_id",
            "stage",
            "schema_version",
            "completion_nonce",
            "result"
        ])
    );
    assert_eq!(
        capability.descriptor.parameters_schema["additionalProperties"],
        json!(false)
    );
    assert!(capability.descriptor.parameters_schema["properties"]["result"].is_object());
}

#[tokio::test]
async fn workflow_result_handler_forwards_to_sink() {
    let sink = Arc::new(RecordingWorkflowSink::default());
    let runtime = runtime_with_workflow_sink(Arc::clone(&sink));

    let output = invoke_with_context(
        &runtime,
        WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID,
        valid_input(),
        execution_context([WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID]),
    )
    .await
    .unwrap();

    assert_eq!(
        output,
        json!({
            "accepted": true,
            "duplicate": false,
            "stage_run_id": "stage-run-1"
        })
    );
    let calls = sink.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].workflow_run_id, "workflow-run-1");
    assert_eq!(calls[0].stage_run_id, "stage-run-1");
    assert_eq!(calls[0].turn_run_id, "turn-run-1");
    assert_eq!(calls[0].stage, "analysis");
    assert_eq!(calls[0].schema_version, "workflow.stage_result.v1");
    assert_eq!(calls[0].completion_nonce, "nonce-1");
    assert_eq!(calls[0].result, json!({"summary": "fixed", "ok": true}));
}

#[tokio::test]
async fn workflow_result_handler_rejects_invalid_json_without_calling_sink() {
    let sink = Arc::new(RecordingWorkflowSink::default());
    let runtime = runtime_with_workflow_sink(Arc::clone(&sink));

    let failure = invoke_failure_with_context(
        &runtime,
        WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID,
        json!({
            "workflow_run_id": "workflow-run-1",
            "stage_run_id": 123,
            "turn_run_id": "turn-run-1",
            "stage": "analysis",
            "schema_version": "workflow.stage_result.v1",
            "completion_nonce": "nonce-1",
            "result": {"summary": "must not be forwarded"}
        }),
        execution_context([WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID]),
    )
    .await;

    assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
    assert!(sink.calls().is_empty());
}

#[tokio::test]
async fn workflow_result_handler_sanitizes_validation_failure() {
    let runtime = runtime_with_workflow_sink(Arc::new(ValidationFailingWorkflowSink::default()));

    let failure = invoke_failure_with_context(
        &runtime,
        WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID,
        json!({
            "workflow_run_id": "workflow-run-1",
            "stage_run_id": "stage-run-1",
            "turn_run_id": "turn-run-1",
            "stage": "analysis",
            "schema_version": "workflow.stage_result.v1",
            "completion_nonce": "nonce-1",
            "result": {"raw": "RAW_RESULT_PAYLOAD_DO_NOT_LEAK"}
        }),
        execution_context([WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID]),
    )
    .await;

    assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
    let summary = failure.safe_summary().expect("sanitized summary");
    assert_eq!(summary, "stage result failed schema validation");
    assert!(!summary.contains("RAW_RESULT_PAYLOAD_DO_NOT_LEAK"));
}

#[test]
fn default_builtin_package_does_not_declare_workflow_result() {
    let package = builtin_first_party_package().unwrap();

    assert!(
        !package
            .capabilities
            .iter()
            .any(|candidate| candidate.id.as_str() == WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID)
    );
    assert!(
        !package
            .manifest
            .capabilities
            .iter()
            .any(|candidate| candidate.id.as_str() == WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID)
    );
}

#[test]
fn workflow_sink_package_declares_result_capability() {
    let package = builtin_first_party_package_with_workflow_stage_result().unwrap();
    let descriptor = package
        .capabilities
        .iter()
        .find(|candidate| candidate.id.as_str() == WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID)
        .expect("workflow result descriptor");

    assert_eq!(descriptor.effects, vec![EffectKind::DispatchCapability]);
    assert_eq!(descriptor.default_permission, PermissionMode::Allow);
    assert!(
        package.manifest.capabilities.iter().any(|candidate| {
            candidate.id.as_str() == WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID
        })
    );
}

#[test]
fn default_builtin_handlers_do_not_register_workflow_result_sink() {
    let handlers =
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap();

    assert!(!handlers.contains_handler(&capability_id(WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID)));
}

#[test]
fn workflow_sink_handlers_registers_result_capability() {
    let handlers = builtin_first_party_handlers_with_workflow_stage_result_sink(
        Arc::new(InMemoryTriggerRepository::default()),
        Arc::new(RecordingWorkflowSink::default()),
    )
    .unwrap();

    assert!(handlers.contains_handler(&capability_id(WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID)));
}

#[derive(Default)]
struct RecordingWorkflowSink {
    calls: Mutex<Vec<ReportWorkflowStageResultInput>>,
}

impl RecordingWorkflowSink {
    fn calls(&self) -> Vec<ReportWorkflowStageResultInput> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl WorkflowStageResultSink for RecordingWorkflowSink {
    async fn report_stage_result(
        &self,
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        self.calls.lock().unwrap().push(input.clone());
        Ok(WorkflowStageResultAck {
            accepted: true,
            duplicate: false,
            stage_run_id: input.stage_run_id,
        })
    }
}

#[derive(Default)]
struct ValidationFailingWorkflowSink {
    calls: Mutex<usize>,
}

#[async_trait]
impl WorkflowStageResultSink for ValidationFailingWorkflowSink {
    async fn report_stage_result(
        &self,
        _input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        *self.calls.lock().unwrap() += 1;
        Err(WorkflowStageResultSinkError::ValidationFailed {
            reason: "stage result failed schema validation".to_string(),
        })
    }
}

fn runtime_with_workflow_sink<T>(sink: Arc<T>) -> impl HostRuntime
where
    T: WorkflowStageResultSink + 'static,
{
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package_with_workflow_stage_result().unwrap())
        .unwrap();
    HostRuntimeServices::new(
        Arc::new(registry),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers_with_workflow_stage_result_sink(
            Arc::new(InMemoryTriggerRepository::default()),
            sink,
        )
        .unwrap(),
    ))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .with_runtime_policy(local_dev_policy())
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

async fn invoke_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> Result<Value, RuntimeFailureKind> {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
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

async fn invoke_failure_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> RuntimeCapabilityFailure {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(capability),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

fn valid_input() -> Value {
    json!({
        "workflow_run_id": "workflow-run-1",
        "stage_run_id": "stage-run-1",
        "turn_run_id": "turn-run-1",
        "stage": "analysis",
        "schema_version": "workflow.stage_result.v1",
        "completion_nonce": "nonce-1",
        "result": {"summary": "fixed", "ok": true}
    })
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn provider_id() -> ExtensionId {
    ExtensionId::new("builtin").unwrap()
}

fn execution_context<I>(grants: I) -> ExecutionContext
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let capability_set = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|grant| dispatch_grant(grant.as_ref()))
            .collect(),
    };
    ExecutionContext::local_default(
        ironclaw_host_api::UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        ironclaw_host_api::MountView::default(),
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
            allowed_effects: builtin_effects(),
            mounts: ironclaw_host_api::MountView::default(),
            network: ironclaw_host_api::NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn builtin_effects() -> Vec<EffectKind> {
    vec![EffectKind::DispatchCapability]
}

fn provider_trust() -> BTreeMap<ExtensionId, TrustDecision> {
    BTreeMap::from([(provider_id(), trust_decision())])
}

fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            builtin_effects(),
            None,
        ),
    ]))])
    .unwrap()
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: builtin_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}

fn local_dev_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}
