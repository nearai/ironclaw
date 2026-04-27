use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_extensions::*;
use ironclaw_host_api::*;
use ironclaw_processes::*;
use ironclaw_run_state::*;
use serde_json::json;

#[tokio::test]
async fn capability_host_uses_obligation_handler_before_dispatch() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = ObligatingAuthorizer::audit_before();
    let handler = RecordingObligationHandler::audit_before_only();
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet::default());

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "handled"}),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(dispatcher.has_request());
    let records = handler.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].phase, CapabilityObligationPhase::Invoke);
    assert_eq!(records[0].obligations, vec![Obligation::AuditBefore]);
}

#[tokio::test]
async fn capability_host_still_fails_closed_when_handler_rejects_obligations() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = ObligatingAuthorizer::redact_output();
    let handler = RecordingObligationHandler::audit_before_only();
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "must not dispatch"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(!dispatcher.has_request());
    assert_eq!(handler.records().len(), 1);
}

#[tokio::test]
async fn capability_host_uses_obligation_handler_before_process_start() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = ObligatingAuthorizer::audit_before();
    let observed = Arc::new(AtomicBool::new(false));
    let handler = FlaggingObligationHandler {
        observed: Arc::clone(&observed),
    };
    let process_manager = OrderingProcessManager {
        obligation_observed: Arc::clone(&observed),
    };
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_obligation_handler(&handler)
        .with_process_manager(&process_manager);
    let context = execution_context(CapabilitySet::default());

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "spawn after obligations"}),
        })
        .await
        .unwrap();

    assert_eq!(result.process.status, ProcessStatus::Running);
    assert!(observed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn capability_host_uses_obligation_handler_on_approved_resume() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved obligation"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();
    let blocked = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    let approval_request_id = blocked.approval_request_id.unwrap();
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();
    let handler = RecordingObligationHandler::audit_before_only();
    let resume_authorizer = ObligatingAuthorizer::audit_before();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases)
        .with_obligation_handler(&handler);

    let result = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate,
            input,
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert_eq!(
        handler.records()[0].phase,
        CapabilityObligationPhase::Resume
    );
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Consumed
    );
}

#[derive(Default)]
struct RecordingDispatcher {
    request: Mutex<Option<CapabilityDispatchRequest>>,
}

impl RecordingDispatcher {
    fn has_request(&self) -> bool {
        self.request.lock().unwrap().is_some()
    }
}

#[async_trait]
impl CapabilityDispatcher for RecordingDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        *self.request.lock().unwrap() = Some(request.clone());
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

struct ObligatingAuthorizer {
    obligations: Vec<Obligation>,
}

impl ObligatingAuthorizer {
    fn audit_before() -> Self {
        Self {
            obligations: vec![Obligation::AuditBefore],
        }
    }

    fn redact_output() -> Self {
        Self {
            obligations: vec![Obligation::RedactOutput],
        }
    }
}

#[async_trait]
impl CapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: self.obligations.clone(),
        }
    }

    async fn authorize_spawn(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: self.obligations.clone(),
        }
    }
}

#[derive(Default)]
struct RecordingObligationHandler {
    records: Mutex<Vec<ObligationRecord>>,
}

impl RecordingObligationHandler {
    fn audit_before_only() -> Self {
        Self::default()
    }

    fn records(&self) -> Vec<ObligationRecord> {
        self.records.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ObligationRecord {
    phase: CapabilityObligationPhase,
    obligations: Vec<Obligation>,
}

#[async_trait]
impl CapabilityObligationHandler for RecordingObligationHandler {
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        self.records.lock().unwrap().push(ObligationRecord {
            phase: request.phase,
            obligations: request.obligations.to_vec(),
        });
        if request
            .obligations
            .iter()
            .all(|obligation| matches!(obligation, Obligation::AuditBefore))
        {
            return Ok(());
        }
        Err(CapabilityObligationError::Unsupported {
            obligations: request.obligations.to_vec(),
        })
    }
}

struct FlaggingObligationHandler {
    observed: Arc<AtomicBool>,
}

#[async_trait]
impl CapabilityObligationHandler for FlaggingObligationHandler {
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        assert_eq!(request.phase, CapabilityObligationPhase::Spawn);
        self.observed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct OrderingProcessManager {
    obligation_observed: Arc<AtomicBool>,
}

#[async_trait]
impl ProcessManager for OrderingProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        assert!(
            self.obligation_observed.load(Ordering::SeqCst),
            "obligation handler must run before process start"
        );
        Ok(ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        })
    }
}

struct ApprovalAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for ApprovalAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: CorrelationId::new(),
                requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
                action: Box::new(Action::Dispatch {
                    capability: CapabilityId::new("echo.say").unwrap(),
                    estimated_resources: ResourceEstimate::default(),
                }),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants,
        mounts: MountView::default(),
        resource_scope,
    }
}

const WASM_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
