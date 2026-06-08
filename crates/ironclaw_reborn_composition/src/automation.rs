use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, CorrelationId, EffectKind,
    ExecutionContext, ExtensionId, GrantConstraints, InvocationId, MountView, NetworkPolicy,
    Principal, ResourceEstimate, ResourceScope, RuntimeKind, TrustClass,
};
use ironclaw_host_runtime::{
    HostRuntime, HostRuntimeError, RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind, TRIGGER_LIST_CAPABILITY_ID,
};
use ironclaw_product_workflow::{
    AutomationListRequest, AutomationProductFacade, ProductAgentBoundCaller, RebornAutomationInfo,
    RebornAutomationRecentRunInfo, RebornAutomationRunStatus, RebornAutomationSource,
    RebornAutomationState, RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use serde::Deserialize;
use serde_json::{Value, json};

const AUTOMATION_BACKEND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct RebornAutomationProductFacade {
    host_runtime: Arc<dyn HostRuntime>,
    backend_timeout: Duration,
}

impl std::fmt::Debug for RebornAutomationProductFacade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornAutomationProductFacade")
            .field("host_runtime", &"Arc<dyn HostRuntime>")
            .finish()
    }
}

impl RebornAutomationProductFacade {
    pub(crate) fn new(host_runtime: Arc<dyn HostRuntime>) -> Self {
        Self {
            host_runtime,
            backend_timeout: AUTOMATION_BACKEND_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_backend_timeout(host_runtime: Arc<dyn HostRuntime>, backend_timeout: Duration) -> Self {
        Self {
            host_runtime,
            backend_timeout,
        }
    }

    async fn invoke_trigger(
        &self,
        caller: ProductAgentBoundCaller,
        capability_id: &'static str,
        input: Value,
    ) -> Result<Value, RebornServicesError> {
        let context = trigger_execution_context(&caller, capability_id)?;
        let request = RuntimeCapabilityRequest::new(
            context,
            CapabilityId::new(capability_id).map_err(|_| internal_invariant())?,
            ResourceEstimate::default(),
            input,
            trigger_trust_decision(),
        );

        match tokio::time::timeout(
            self.backend_timeout,
            self.host_runtime.invoke_capability(request),
        )
        .await
        {
            Err(_) => Err(services_error(
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::ServiceUnavailable,
                503,
                true,
            )),
            Ok(Ok(RuntimeCapabilityOutcome::Completed(completed))) => Ok(completed.output),
            Ok(Ok(RuntimeCapabilityOutcome::ApprovalRequired(_))) => Err(services_error(
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::BlockedApproval,
                409,
                false,
            )),
            Ok(Ok(RuntimeCapabilityOutcome::AuthRequired(_))) => Err(services_error(
                RebornServicesErrorCode::Forbidden,
                RebornServicesErrorKind::BlockedAuthentication,
                403,
                false,
            )),
            Ok(Ok(RuntimeCapabilityOutcome::ResourceBlocked(_))) => Err(services_error(
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::BlockedResource,
                503,
                true,
            )),
            Ok(Ok(RuntimeCapabilityOutcome::SpawnedProcess(_))) => Err(services_error(
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::ServiceUnavailable,
                503,
                true,
            )),
            Ok(Ok(RuntimeCapabilityOutcome::Failed(failure))) => Err(map_runtime_failure(failure)),
            Ok(Ok(RuntimeCapabilityOutcome::Unknown(_))) => Err(services_error(
                RebornServicesErrorCode::Internal,
                RebornServicesErrorKind::Internal,
                500,
                false,
            )),
            Ok(Err(error)) => Err(map_host_runtime_error(error)),
        }
    }
}

#[async_trait::async_trait]
impl AutomationProductFacade for RebornAutomationProductFacade {
    async fn list_automations(
        &self,
        caller: ProductAgentBoundCaller,
        request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
        let output = self
            .invoke_trigger(
                caller,
                TRIGGER_LIST_CAPABILITY_ID,
                json!({
                    "limit": request.limit,
                    "run_limit": request.run_limit,
                }),
            )
            .await?;
        parse_list_automations_output(output)
    }
}

#[derive(Debug, Deserialize)]
struct RawAutomationListEnvelope {
    triggers: Vec<RawAutomationRecord>,
}

#[derive(Debug, Deserialize)]
struct RawAutomationRecord {
    trigger_id: String,
    name: String,
    schedule: RawAutomationSchedule,
    state: RebornAutomationState,
    #[serde(default)]
    next_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_status: Option<RebornAutomationRunStatus>,
    #[serde(default)]
    recent_runs: Vec<RebornAutomationRecentRunInfo>,
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum RawAutomationSchedule {
    Cron {
        expression: String,
    },
    #[serde(other)]
    Unknown,
}

fn parse_list_automations_output(
    mut output: Value,
) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
    sanitize_automation_list_output(&mut output);
    let envelope: RawAutomationListEnvelope = serde_json::from_value(output).map_err(|error| {
        tracing::debug!(
            error = %error,
            "malformed automation list output from host runtime"
        );
        internal_invariant()
    })?;
    Ok(envelope
        .triggers
        .into_iter()
        .filter_map(automation_info)
        .collect())
}

fn automation_info(record: RawAutomationRecord) -> Option<RebornAutomationInfo> {
    Some(RebornAutomationInfo {
        automation_id: record.trigger_id,
        name: record.name,
        source: automation_source(record.schedule)?,
        state: record.state,
        next_run_at: record.next_run_at,
        last_run_at: record.last_run_at,
        last_status: record.last_status,
        recent_runs: record.recent_runs,
        is_active: record.is_active,
        created_at: record.created_at,
    })
}

fn automation_source(schedule: RawAutomationSchedule) -> Option<RebornAutomationSource> {
    match schedule {
        RawAutomationSchedule::Cron { expression } => {
            Some(RebornAutomationSource::Schedule { cron: expression })
        }
        RawAutomationSchedule::Unknown => None,
    }
}

fn sanitize_automation_list_output(output: &mut Value) {
    let Some(triggers) = output.get_mut("triggers").and_then(Value::as_array_mut) else {
        return;
    };
    for trigger in triggers {
        let Some(trigger_object) = trigger.as_object_mut() else {
            continue;
        };
        let status = match trigger_object.get("last_status").and_then(Value::as_str) {
            Some("ok") => Value::String("ok".to_string()),
            Some("error") => Value::String("error".to_string()),
            _ => Value::Null,
        };
        let state = match trigger_object.get("state").and_then(Value::as_str) {
            Some(state) => Value::String(state.to_string()),
            None => Value::String("unknown".to_string()),
        };
        trigger_object.insert("last_status".to_string(), status);
        trigger_object.insert("state".to_string(), state);
        sanitize_recent_run_statuses(trigger_object);
    }
}

fn sanitize_recent_run_statuses(trigger_object: &mut serde_json::Map<String, Value>) {
    let Some(recent_runs) = trigger_object
        .get_mut("recent_runs")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for run in recent_runs {
        let Some(run_object) = run.as_object_mut() else {
            continue;
        };
        let status = match run_object.get("status").and_then(Value::as_str) {
            Some("running") => Value::String("running".to_string()),
            Some("ok") => Value::String("ok".to_string()),
            Some("error") => Value::String("error".to_string()),
            _ => Value::String("unknown".to_string()),
        };
        run_object.insert("status".to_string(), status);
    }
}

fn trigger_execution_context(
    caller: &ProductAgentBoundCaller,
    capability_id: &str,
) -> Result<ExecutionContext, RebornServicesError> {
    let extension_id = automation_extension_id()?;
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: Some(caller.agent_id.clone()),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let grants = CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: CapabilityId::new(capability_id).map_err(|_| internal_invariant())?,
            grantee: Principal::Extension(extension_id.clone()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: trigger_allowed_effects(),
                mounts: MountView::new(Vec::new()).map_err(|_| internal_invariant())?,
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    };
    let context = ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: Some(caller.agent_id.clone()),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        extension_id,
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::UserTrusted,
        grants,
        mounts: MountView::new(Vec::new()).map_err(|_| internal_invariant())?,
        resource_scope,
    };
    context.validate().map_err(|_| internal_invariant())?;
    Ok(context)
}

fn trigger_trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: trigger_allowed_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}

fn trigger_allowed_effects() -> Vec<EffectKind> {
    vec![EffectKind::DispatchCapability]
}

fn map_runtime_failure(failure: RuntimeCapabilityFailure) -> RebornServicesError {
    match failure.kind {
        RuntimeFailureKind::InvalidInput => services_error(
            RebornServicesErrorCode::InvalidRequest,
            RebornServicesErrorKind::Validation,
            400,
            false,
        ),
        RuntimeFailureKind::Authorization | RuntimeFailureKind::PolicyDenied => services_error(
            RebornServicesErrorCode::Forbidden,
            RebornServicesErrorKind::ParticipantDenied,
            403,
            false,
        ),
        RuntimeFailureKind::Cancelled => services_error(
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Conflict,
            409,
            false,
        ),
        RuntimeFailureKind::Unavailable
        | RuntimeFailureKind::Backend
        | RuntimeFailureKind::Dispatcher
        | RuntimeFailureKind::Internal
        | RuntimeFailureKind::MissingRuntime
        | RuntimeFailureKind::Network
        | RuntimeFailureKind::Process
        | RuntimeFailureKind::Resource
        | RuntimeFailureKind::Transient
        | RuntimeFailureKind::Unknown
        | RuntimeFailureKind::OperationFailed
        | RuntimeFailureKind::OutputTooLarge
        | RuntimeFailureKind::InvalidOutput
        | _ => services_error(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ServiceUnavailable,
            503,
            true,
        ),
    }
}

fn map_host_runtime_error(error: HostRuntimeError) -> RebornServicesError {
    match error {
        HostRuntimeError::InvalidRequest { .. } => services_error(
            RebornServicesErrorCode::Internal,
            RebornServicesErrorKind::Internal,
            500,
            false,
        ),
        HostRuntimeError::Unavailable { .. } => services_error(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ServiceUnavailable,
            503,
            true,
        ),
    }
}

fn automation_extension_id() -> Result<ExtensionId, RebornServicesError> {
    ExtensionId::new("reborn.product.automation").map_err(|_| internal_invariant())
}

fn services_error(
    code: RebornServicesErrorCode,
    kind: RebornServicesErrorKind,
    status_code: u16,
    retryable: bool,
) -> RebornServicesError {
    RebornServicesError {
        code,
        kind,
        status_code,
        retryable,
        field: None,
        validation_code: None,
    }
}

fn internal_invariant() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Internal,
        kind: RebornServicesErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_host_api::{
        AgentId, ApprovalRequestId, ProcessId, ProjectId, SecretHandle, TenantId, TrustClass,
        UserId,
    };
    use ironclaw_host_runtime::{
        HostRuntime, HostRuntimeError, RuntimeApprovalGate, RuntimeAuthGate, RuntimeBlockedReason,
        RuntimeCapabilityCompleted, RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
        RuntimeCapabilityRequest, RuntimeCapabilityUnknown, RuntimeFailureKind, RuntimeGateId,
        RuntimeProcessHandle, RuntimeResourceGate, TRIGGER_LIST_CAPABILITY_ID,
    };
    use ironclaw_product_workflow::{
        AutomationListRequest, AutomationProductFacade, ProductAgentBoundCaller,
        RebornAutomationRecentRunStatus, RebornAutomationRunStatus, RebornAutomationSource,
        RebornAutomationState, RebornServicesErrorCode, RebornServicesErrorKind,
    };
    use serde_json::{Value, json};
    use tokio::sync::Mutex;

    use super::RebornAutomationProductFacade;

    #[tokio::test]
    async fn automation_facade_preserves_caller_scope_and_capability_path() {
        let runtime = Arc::new(RecordingHostRuntime::default());
        let facade = RebornAutomationProductFacade::new(runtime.clone());
        let caller = caller();

        let automations = facade
            .list_automations(caller.clone(), automation_list_request(25, 10))
            .await
            .expect("trigger list output");

        assert_eq!(automations.len(), 1);
        assert_eq!(automations[0].automation_id, "trigger-listed");
        assert_eq!(
            automations[0].source,
            RebornAutomationSource::Schedule {
                cron: "0 9 * * *".to_string()
            }
        );
        assert_eq!(
            automations[0].last_status,
            Some(RebornAutomationRunStatus::Ok)
        );
        assert_eq!(automations[0].recent_runs.len(), 1);
        assert_eq!(
            automations[0].recent_runs[0]
                .run_id
                .map(|run_id| run_id.to_string())
                .as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(
            automations[0].recent_runs[0].thread_id.as_str(),
            "thread-listed"
        );
        assert_eq!(
            automations[0].recent_runs[0].status,
            RebornAutomationRecentRunStatus::Running
        );
        let request = runtime
            .requests
            .lock()
            .await
            .pop()
            .expect("runtime request");
        assert_eq!(request.capability_id.as_str(), TRIGGER_LIST_CAPABILITY_ID);
        assert_eq!(request.context.tenant_id, caller.tenant_id);
        assert_eq!(request.context.user_id, caller.user_id);
        assert_eq!(request.context.agent_id, Some(caller.agent_id.clone()));
        assert_eq!(request.context.project_id, caller.project_id);
        assert_eq!(request.context.resource_scope.tenant_id, caller.tenant_id);
        assert_eq!(request.context.resource_scope.user_id, caller.user_id);
        assert_eq!(
            request.context.resource_scope.agent_id,
            Some(caller.agent_id)
        );
        assert_eq!(request.context.resource_scope.project_id, caller.project_id);
        assert_eq!(request.context.trust, TrustClass::UserTrusted);
        assert_eq!(request.input["limit"], 25);
        assert_eq!(request.input["run_limit"], 10);
    }

    #[tokio::test]
    async fn automation_facade_rejects_malformed_trigger_list_output() {
        let facade = RebornAutomationProductFacade::new(Arc::new(OutputHostRuntime::new(json!({
            "unexpected": true
        }))));

        let error = facade
            .list_automations(caller(), automation_list_request(50, 10))
            .await
            .expect_err("malformed automation output should fail closed");

        assert_eq!(error.code, RebornServicesErrorCode::Internal);
        assert_eq!(error.status_code, 500);
    }

    #[tokio::test]
    async fn automation_facade_rejects_non_array_trigger_list_output() {
        let facade = RebornAutomationProductFacade::new(Arc::new(OutputHostRuntime::new(json!({
            "triggers": {
                "trigger_id": "trigger-listed"
            }
        }))));

        let error = facade
            .list_automations(caller(), automation_list_request(50, 10))
            .await
            .expect_err("malformed automation output should fail closed");

        assert_eq!(error.code, RebornServicesErrorCode::Internal);
        assert_eq!(error.status_code, 500);
    }

    #[tokio::test]
    async fn automation_facade_drops_unallowlisted_status_payloads() {
        let mut trigger =
            raw_automation("trigger-listed", "Daily status", "0 9 * * *", Some("error"))
                .as_object()
                .cloned()
                .expect("object trigger");
        trigger.insert(
            "last_status".to_string(),
            json!({"trace": "internal details", "secret": "token"}),
        );
        let facade = RebornAutomationProductFacade::new(Arc::new(OutputHostRuntime::new(json!({
            "triggers": [Value::Object(trigger)]
        }))));

        let automations = facade
            .list_automations(caller(), automation_list_request(50, 10))
            .await
            .expect("list automations");

        assert_eq!(automations.len(), 1);
        assert_eq!(automations[0].last_status, None);
    }

    #[tokio::test]
    async fn automation_facade_sanitizes_malformed_recent_run_status_payloads() {
        let mut unknown_status = raw_automation(
            "trigger-unknown-status",
            "Unknown run status",
            "0 9 * * *",
            Some("ok"),
        )
        .as_object()
        .cloned()
        .expect("object trigger");
        unknown_status["recent_runs"][0]["status"] = json!("future_state");
        let mut non_string_status = raw_automation(
            "trigger-non-string-status",
            "Non-string run status",
            "0 10 * * *",
            Some("ok"),
        )
        .as_object()
        .cloned()
        .expect("object trigger");
        non_string_status["recent_runs"][0]["status"] = json!({"raw": "backend-only"});
        let facade = RebornAutomationProductFacade::new(Arc::new(OutputHostRuntime::new(json!({
            "triggers": [
                Value::Object(unknown_status),
                Value::Object(non_string_status)
            ]
        }))));

        let automations = facade
            .list_automations(caller(), automation_list_request(50, 10))
            .await
            .expect("list automations");

        assert_eq!(automations.len(), 2);
        assert_eq!(
            automations[0].recent_runs[0].status,
            RebornAutomationRecentRunStatus::Unknown
        );
        assert_eq!(
            automations[1].recent_runs[0].status,
            RebornAutomationRecentRunStatus::Unknown
        );
    }

    #[tokio::test]
    async fn automation_facade_parses_known_and_unknown_states() {
        let mut paused = raw_automation("trigger-paused", "Paused status", "0 9 * * *", Some("ok"))
            .as_object()
            .cloned()
            .expect("object trigger");
        paused.insert("state".to_string(), json!("paused"));
        let mut scheduled = raw_automation(
            "trigger-scheduled",
            "Scheduled status",
            "0 10 * * *",
            Some("ok"),
        )
        .as_object()
        .cloned()
        .expect("object trigger");
        scheduled.insert("state".to_string(), json!("scheduled"));
        let mut completed = raw_automation(
            "trigger-completed",
            "Completed status",
            "0 11 * * *",
            Some("ok"),
        )
        .as_object()
        .cloned()
        .expect("object trigger");
        completed.insert("state".to_string(), json!("completed"));
        let mut non_string = raw_automation(
            "trigger-non-string",
            "Malformed state",
            "0 12 * * *",
            Some("ok"),
        )
        .as_object()
        .cloned()
        .expect("object trigger");
        non_string.insert("state".to_string(), json!({"raw": "backend-only"}));
        let future = json!({
            "trigger_id": "trigger-future",
            "name": "Future status",
            "schedule": {"kind": "cron", "expression": "0 13 * * *"},
            "state": "future_state",
            "next_run_at": "2026-06-03T09:00:00Z",
            "last_run_at": null,
            "last_status": "ok",
            "is_active": true,
            "created_at": "2026-06-02T18:00:00Z"
        });
        let facade = RebornAutomationProductFacade::new(Arc::new(OutputHostRuntime::new(json!({
            "triggers": [
                Value::Object(paused),
                Value::Object(scheduled),
                Value::Object(completed),
                Value::Object(non_string),
                future
            ]
        }))));

        let automations = facade
            .list_automations(caller(), automation_list_request(50, 10))
            .await
            .expect("list automations");

        assert_eq!(automations.len(), 5);
        assert_eq!(automations[0].state, RebornAutomationState::Paused);
        assert_eq!(automations[1].state, RebornAutomationState::Scheduled);
        assert_eq!(automations[2].state, RebornAutomationState::Completed);
        assert_eq!(automations[3].state, RebornAutomationState::Unknown);
        assert_eq!(automations[4].state, RebornAutomationState::Unknown);
    }

    #[tokio::test]
    async fn automation_facade_filters_unknown_future_sources() {
        let facade = RebornAutomationProductFacade::new(Arc::new(OutputHostRuntime::new(json!({
            "triggers": [
                raw_automation("trigger-schedule", "Daily status", "0 9 * * *", Some("ok")),
                {
                    "trigger_id": "trigger-webhook",
                    "name": "Future webhook",
                    "schedule": {"kind": "webhook"},
                    "state": "active",
                    "last_status": "ok",
                    "is_active": false
                }
            ]
        }))));

        let automations = facade
            .list_automations(caller(), automation_list_request(50, 10))
            .await
            .expect("list automations");

        assert_eq!(automations.len(), 1);
        assert_eq!(automations[0].automation_id, "trigger-schedule");
    }

    #[tokio::test]
    async fn automation_facade_rejects_malformed_trigger_records() {
        let facade = RebornAutomationProductFacade::new(Arc::new(OutputHostRuntime::new(json!({
            "triggers": [{
                "name": "Missing trigger id",
                "schedule": {"kind": "cron", "expression": "0 9 * * *"},
                "state": "active",
                "last_status": "ok",
                "is_active": true
            }]
        }))));

        let error = facade
            .list_automations(caller(), automation_list_request(50, 10))
            .await
            .expect_err("malformed record should fail closed");

        assert_eq!(error.code, RebornServicesErrorCode::Internal);
        assert_eq!(error.status_code, 500);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn automation_facade_redacts_runtime_failure_messages() {
        let runtime = Arc::new(FailingHostRuntime::new(RuntimeFailureKind::Internal));
        let facade = RebornAutomationProductFacade::new(runtime);
        let caller = caller();

        let error = facade
            .list_automations(caller, automation_list_request(10, 5))
            .await
            .expect_err("runtime failure should map to services error");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
        assert!(!format!("{error:?}").contains("redacted runtime details"));
    }

    #[tokio::test]
    async fn automation_facade_times_out_stalled_runtime() {
        let facade = RebornAutomationProductFacade::with_backend_timeout(
            Arc::new(HangingHostRuntime),
            std::time::Duration::from_millis(1),
        );

        let error = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            facade.list_automations(caller(), automation_list_request(10, 5)),
        )
        .await
        .expect("facade timeout should complete promptly")
        .expect_err("stalled runtime should time out");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
    }

    #[tokio::test]
    async fn automation_facade_maps_blocked_and_unknown_outcomes() {
        let capability_id =
            ironclaw_host_api::CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID).expect("capability");
        let cases = [
            (
                RuntimeCapabilityOutcome::ApprovalRequired(RuntimeApprovalGate {
                    approval_request_id: ApprovalRequestId::new(),
                    capability_id: capability_id.clone(),
                    reason: RuntimeBlockedReason::ApprovalRequired,
                }),
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::BlockedApproval,
                409,
                false,
            ),
            (
                RuntimeCapabilityOutcome::AuthRequired(RuntimeAuthGate {
                    gate_id: RuntimeGateId::new(),
                    capability_id: capability_id.clone(),
                    reason: RuntimeBlockedReason::AuthRequired,
                    required_secrets: vec![SecretHandle::new("automation_token").expect("secret")],
                    credential_requirements: Vec::new(),
                }),
                RebornServicesErrorCode::Forbidden,
                RebornServicesErrorKind::BlockedAuthentication,
                403,
                false,
            ),
            (
                RuntimeCapabilityOutcome::ResourceBlocked(RuntimeResourceGate {
                    gate_id: RuntimeGateId::new(),
                    capability_id: capability_id.clone(),
                    reason: RuntimeBlockedReason::ResourceLimit,
                    estimate: ironclaw_host_api::ResourceEstimate::default(),
                }),
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::BlockedResource,
                503,
                true,
            ),
            (
                RuntimeCapabilityOutcome::SpawnedProcess(RuntimeProcessHandle {
                    process_id: ProcessId::new(),
                    capability_id: capability_id.clone(),
                }),
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::ServiceUnavailable,
                503,
                true,
            ),
            (
                RuntimeCapabilityOutcome::Unknown(RuntimeCapabilityUnknown {
                    capability_id,
                    kind: "future_outcome".to_string(),
                    message: Some("internal details".to_string()),
                }),
                RebornServicesErrorCode::Internal,
                RebornServicesErrorKind::Internal,
                500,
                false,
            ),
        ];

        for (outcome, code, kind, status_code, retryable) in cases {
            let facade =
                RebornAutomationProductFacade::new(Arc::new(OutcomeHostRuntime::new(outcome)));

            let error = facade
                .list_automations(caller(), automation_list_request(50, 10))
                .await
                .expect_err("outcome should map to services error");

            assert_eq!(error.code, code);
            assert_eq!(error.kind, kind);
            assert_eq!(error.status_code, status_code);
            assert_eq!(error.retryable, retryable);
        }
    }

    #[tokio::test]
    async fn automation_facade_maps_runtime_failure_branches() {
        let cases = [
            (
                RuntimeFailureKind::InvalidInput,
                RebornServicesErrorCode::InvalidRequest,
                RebornServicesErrorKind::Validation,
                400,
                false,
            ),
            (
                RuntimeFailureKind::Authorization,
                RebornServicesErrorCode::Forbidden,
                RebornServicesErrorKind::ParticipantDenied,
                403,
                false,
            ),
            (
                RuntimeFailureKind::PolicyDenied,
                RebornServicesErrorCode::Forbidden,
                RebornServicesErrorKind::ParticipantDenied,
                403,
                false,
            ),
            (
                RuntimeFailureKind::Cancelled,
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::Conflict,
                409,
                false,
            ),
        ];

        for (failure_kind, code, kind, status_code, retryable) in cases {
            let facade =
                RebornAutomationProductFacade::new(Arc::new(FailingHostRuntime::new(failure_kind)));

            let error = facade
                .list_automations(caller(), automation_list_request(10, 5))
                .await
                .expect_err("runtime failure should map to services error");

            assert_eq!(error.code, code);
            assert_eq!(error.kind, kind);
            assert_eq!(error.status_code, status_code);
            assert_eq!(error.retryable, retryable);
        }
    }

    #[tokio::test]
    async fn automation_facade_maps_host_runtime_errors() {
        let cases = [
            (
                HostRuntimeError::invalid_request("bad runtime request"),
                RebornServicesErrorCode::Internal,
                RebornServicesErrorKind::Internal,
                500,
                false,
            ),
            (
                HostRuntimeError::unavailable("runtime down"),
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::ServiceUnavailable,
                503,
                true,
            ),
        ];

        for (host_error, code, kind, status_code, retryable) in cases {
            let facade =
                RebornAutomationProductFacade::new(Arc::new(ErrorHostRuntime::new(host_error)));

            let error = facade
                .list_automations(caller(), automation_list_request(10, 5))
                .await
                .expect_err("host runtime error should map to services error");

            assert_eq!(error.code, code);
            assert_eq!(error.kind, kind);
            assert_eq!(error.status_code, status_code);
            assert_eq!(error.retryable, retryable);
        }
    }

    fn caller() -> ProductAgentBoundCaller {
        ProductAgentBoundCaller {
            tenant_id: TenantId::new("tenant-alpha").expect("valid tenant"),
            user_id: UserId::new("user-alpha").expect("valid user"),
            agent_id: AgentId::new("agent-alpha").expect("valid agent"),
            project_id: Some(ProjectId::new("project-alpha").expect("valid project")),
        }
    }

    fn automation_list_request(limit: usize, run_limit: usize) -> AutomationListRequest {
        AutomationListRequest { limit, run_limit }
    }

    fn raw_automation(
        trigger_id: &str,
        name: impl Into<String>,
        cron: impl Into<String>,
        last_status: Option<&str>,
    ) -> Value {
        json!({
            "trigger_id": trigger_id,
            "name": name.into(),
            "schedule": {
                "kind": "cron",
                "expression": cron.into()
            },
            "state": "active",
            "next_run_at": "2026-06-03T09:00:00Z",
            "last_run_at": null,
            "last_status": last_status,
            "is_active": true,
            "created_at": "2026-06-02T18:00:00Z",
            "recent_runs": [{
                "run_id": "11111111-1111-1111-1111-111111111111",
                "thread_id": "thread-listed",
                "fire_slot": "2026-06-03T09:00:00Z",
                "status": "running",
                "submitted_at": "2026-06-03T09:00:01Z",
                "completed_at": null
            }]
        })
    }

    struct RecordingHostRuntime {
        requests: Mutex<Vec<RuntimeCapabilityRequest>>,
    }

    impl Default for RecordingHostRuntime {
        fn default() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl HostRuntime for RecordingHostRuntime {
        async fn invoke_capability(
            &self,
            request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.requests.lock().await.push(request.clone());
            Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                RuntimeCapabilityCompleted {
                    capability_id: request.capability_id,
                    output: json!({
                        "triggers": [
                            raw_automation(
                                "trigger-listed",
                                "Daily status",
                                "0 9 * * *",
                                Some("ok"),
                            )
                        ]
                    }),
                    display_preview: None,
                    usage: ironclaw_host_api::ResourceUsage::default(),
                },
            )))
        }

        async fn resume_capability(
            &self,
            _request: ironclaw_host_runtime::RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("resume capability is not used in automation facade tests")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<ironclaw_host_runtime::VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("visible capabilities are not used in automation facade tests")
        }

        async fn cancel_work(
            &self,
            _request: ironclaw_host_runtime::CancelRuntimeWorkRequest,
        ) -> Result<ironclaw_host_runtime::CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("cancel work is not used in automation facade tests")
        }

        async fn runtime_status(
            &self,
            _request: ironclaw_host_runtime::RuntimeStatusRequest,
        ) -> Result<ironclaw_host_runtime::HostRuntimeStatus, HostRuntimeError> {
            unreachable!("runtime status is not used in automation facade tests")
        }

        async fn health(
            &self,
        ) -> Result<ironclaw_host_runtime::HostRuntimeHealth, HostRuntimeError> {
            unreachable!("health is not used in automation facade tests")
        }
    }

    struct OutputHostRuntime {
        output: Value,
    }

    impl OutputHostRuntime {
        fn new(output: Value) -> Self {
            Self { output }
        }
    }

    #[async_trait]
    impl HostRuntime for OutputHostRuntime {
        async fn invoke_capability(
            &self,
            request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                RuntimeCapabilityCompleted {
                    capability_id: request.capability_id,
                    output: self.output.clone(),
                    display_preview: None,
                    usage: ironclaw_host_api::ResourceUsage::default(),
                },
            )))
        }

        async fn resume_capability(
            &self,
            _request: ironclaw_host_runtime::RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("resume capability is not used in automation facade tests")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<ironclaw_host_runtime::VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("visible capabilities are not used in automation facade tests")
        }

        async fn cancel_work(
            &self,
            _request: ironclaw_host_runtime::CancelRuntimeWorkRequest,
        ) -> Result<ironclaw_host_runtime::CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("cancel work is not used in automation facade tests")
        }

        async fn runtime_status(
            &self,
            _request: ironclaw_host_runtime::RuntimeStatusRequest,
        ) -> Result<ironclaw_host_runtime::HostRuntimeStatus, HostRuntimeError> {
            unreachable!("runtime status is not used in automation facade tests")
        }

        async fn health(
            &self,
        ) -> Result<ironclaw_host_runtime::HostRuntimeHealth, HostRuntimeError> {
            unreachable!("health is not used in automation facade tests")
        }
    }

    struct FailingHostRuntime {
        kind: RuntimeFailureKind,
    }

    impl FailingHostRuntime {
        fn new(kind: RuntimeFailureKind) -> Self {
            Self { kind }
        }
    }

    struct OutcomeHostRuntime {
        outcome: RuntimeCapabilityOutcome,
    }

    impl OutcomeHostRuntime {
        fn new(outcome: RuntimeCapabilityOutcome) -> Self {
            Self { outcome }
        }
    }

    #[async_trait]
    impl HostRuntime for OutcomeHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            Ok(self.outcome.clone())
        }

        async fn resume_capability(
            &self,
            _request: ironclaw_host_runtime::RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("resume capability is not used in automation facade tests")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<ironclaw_host_runtime::VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("visible capabilities are not used in automation facade tests")
        }

        async fn cancel_work(
            &self,
            _request: ironclaw_host_runtime::CancelRuntimeWorkRequest,
        ) -> Result<ironclaw_host_runtime::CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("cancel work is not used in automation facade tests")
        }

        async fn runtime_status(
            &self,
            _request: ironclaw_host_runtime::RuntimeStatusRequest,
        ) -> Result<ironclaw_host_runtime::HostRuntimeStatus, HostRuntimeError> {
            unreachable!("runtime status is not used in automation facade tests")
        }

        async fn health(
            &self,
        ) -> Result<ironclaw_host_runtime::HostRuntimeHealth, HostRuntimeError> {
            unreachable!("health is not used in automation facade tests")
        }
    }

    struct ErrorHostRuntime {
        error: HostRuntimeError,
    }

    impl ErrorHostRuntime {
        fn new(error: HostRuntimeError) -> Self {
            Self { error }
        }
    }

    #[async_trait]
    impl HostRuntime for ErrorHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            Err(self.error.clone())
        }

        async fn resume_capability(
            &self,
            _request: ironclaw_host_runtime::RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("resume capability is not used in automation facade tests")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<ironclaw_host_runtime::VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("visible capabilities are not used in automation facade tests")
        }

        async fn cancel_work(
            &self,
            _request: ironclaw_host_runtime::CancelRuntimeWorkRequest,
        ) -> Result<ironclaw_host_runtime::CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("cancel work is not used in automation facade tests")
        }

        async fn runtime_status(
            &self,
            _request: ironclaw_host_runtime::RuntimeStatusRequest,
        ) -> Result<ironclaw_host_runtime::HostRuntimeStatus, HostRuntimeError> {
            unreachable!("runtime status is not used in automation facade tests")
        }

        async fn health(
            &self,
        ) -> Result<ironclaw_host_runtime::HostRuntimeHealth, HostRuntimeError> {
            unreachable!("health is not used in automation facade tests")
        }
    }

    struct HangingHostRuntime;

    #[async_trait]
    impl HostRuntime for HangingHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            std::future::pending().await
        }

        async fn resume_capability(
            &self,
            _request: ironclaw_host_runtime::RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("resume capability is not used in automation facade tests")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<ironclaw_host_runtime::VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("visible capabilities are not used in automation facade tests")
        }

        async fn cancel_work(
            &self,
            _request: ironclaw_host_runtime::CancelRuntimeWorkRequest,
        ) -> Result<ironclaw_host_runtime::CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("cancel work is not used in automation facade tests")
        }

        async fn runtime_status(
            &self,
            _request: ironclaw_host_runtime::RuntimeStatusRequest,
        ) -> Result<ironclaw_host_runtime::HostRuntimeStatus, HostRuntimeError> {
            unreachable!("runtime status is not used in automation facade tests")
        }

        async fn health(
            &self,
        ) -> Result<ironclaw_host_runtime::HostRuntimeHealth, HostRuntimeError> {
            unreachable!("health is not used in automation facade tests")
        }
    }

    #[async_trait]
    impl HostRuntime for FailingHostRuntime {
        async fn invoke_capability(
            &self,
            request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            Ok(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                capability_id: request.capability_id,
                kind: self.kind,
                message: Some("redacted runtime details".to_string()),
            }))
        }

        async fn resume_capability(
            &self,
            _request: ironclaw_host_runtime::RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("resume capability is not used in automation facade tests")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<ironclaw_host_runtime::VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("visible capabilities are not used in automation facade tests")
        }

        async fn cancel_work(
            &self,
            _request: ironclaw_host_runtime::CancelRuntimeWorkRequest,
        ) -> Result<ironclaw_host_runtime::CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("cancel work is not used in automation facade tests")
        }

        async fn runtime_status(
            &self,
            _request: ironclaw_host_runtime::RuntimeStatusRequest,
        ) -> Result<ironclaw_host_runtime::HostRuntimeStatus, HostRuntimeError> {
            unreachable!("runtime status is not used in automation facade tests")
        }

        async fn health(
            &self,
        ) -> Result<ironclaw_host_runtime::HostRuntimeHealth, HostRuntimeError> {
            unreachable!("health is not used in automation facade tests")
        }
    }
}
