use std::sync::Arc;

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
    AutomationProductFacade, RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    WebUiAuthenticatedCaller,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct RebornWebuiAutomationFacade {
    host_runtime: Arc<dyn HostRuntime>,
}

impl std::fmt::Debug for RebornWebuiAutomationFacade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornWebuiAutomationFacade")
            .field("host_runtime", &true)
            .finish()
    }
}

impl RebornWebuiAutomationFacade {
    pub(crate) fn new(host_runtime: Arc<dyn HostRuntime>) -> Self {
        Self { host_runtime }
    }

    pub async fn list_triggers(
        &self,
        caller: WebUiAuthenticatedCaller,
        limit: Option<usize>,
    ) -> Result<Value, RebornServicesError> {
        self.invoke_trigger(
            caller,
            TRIGGER_LIST_CAPABILITY_ID,
            json!({
                "limit": limit,
            }),
        )
        .await
    }

    async fn invoke_trigger(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability_id: &'static str,
        input: Value,
    ) -> Result<Value, RebornServicesError> {
        let context = trigger_execution_context(&caller, capability_id)?;
        let request = RuntimeCapabilityRequest::new(
            context,
            CapabilityId::new(capability_id).map_err(|_| internal_invariant())?,
            ResourceEstimate::default(),
            input,
            trigger_trust_decision(capability_id),
        );

        match self.host_runtime.invoke_capability(request).await {
            Ok(RuntimeCapabilityOutcome::Completed(completed)) => Ok(completed.output),
            Ok(RuntimeCapabilityOutcome::ApprovalRequired(_)) => Err(services_error(
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::BlockedApproval,
                409,
                false,
            )),
            Ok(RuntimeCapabilityOutcome::AuthRequired(_)) => Err(services_error(
                RebornServicesErrorCode::Forbidden,
                RebornServicesErrorKind::BlockedAuthentication,
                403,
                false,
            )),
            Ok(RuntimeCapabilityOutcome::ResourceBlocked(_)) => Err(services_error(
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::BlockedResource,
                503,
                true,
            )),
            Ok(RuntimeCapabilityOutcome::SpawnedProcess(_)) => Err(services_error(
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::ServiceUnavailable,
                503,
                true,
            )),
            Ok(RuntimeCapabilityOutcome::Failed(failure)) => Err(map_runtime_failure(failure)),
            Ok(RuntimeCapabilityOutcome::Unknown(_)) => Err(services_error(
                RebornServicesErrorCode::Internal,
                RebornServicesErrorKind::Internal,
                500,
                false,
            )),
            Err(error) => Err(map_host_runtime_error(error)),
        }
    }
}

#[async_trait::async_trait]
impl AutomationProductFacade for RebornWebuiAutomationFacade {
    async fn list_automations(
        &self,
        caller: WebUiAuthenticatedCaller,
        limit: Option<usize>,
    ) -> Result<Value, RebornServicesError> {
        self.list_triggers(caller, limit).await
    }
}

fn trigger_execution_context(
    caller: &WebUiAuthenticatedCaller,
    capability_id: &str,
) -> Result<ExecutionContext, RebornServicesError> {
    let extension_id = automation_extension_id()?;
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: caller.agent_id.clone(),
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
                allowed_effects: trigger_allowed_effects(capability_id),
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
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        extension_id,
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::FirstParty,
        grants,
        mounts: MountView::new(Vec::new()).map_err(|_| internal_invariant())?,
        resource_scope,
    };
    context.validate().map_err(|_| internal_invariant())?;
    Ok(context)
}

fn trigger_trust_decision(capability_id: &str) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: trigger_allowed_effects(capability_id),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}

fn trigger_allowed_effects(capability_id: &str) -> Vec<EffectKind> {
    let _ = capability_id;
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
    ExtensionId::new("reborn.webui.automation").map_err(|_| internal_invariant())
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
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
    use ironclaw_host_runtime::{
        HostRuntime, HostRuntimeError, RuntimeCapabilityCompleted, RuntimeCapabilityFailure,
        RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind,
        TRIGGER_LIST_CAPABILITY_ID,
    };
    use ironclaw_product_workflow::{
        RebornServicesErrorCode, RebornServicesErrorKind, WebUiAuthenticatedCaller,
    };
    use serde_json::json;
    use tokio::sync::Mutex;

    use super::RebornWebuiAutomationFacade;

    #[tokio::test]
    async fn automation_facade_preserves_caller_scope_and_capability_path() {
        let runtime = Arc::new(RecordingHostRuntime::default());
        let facade = RebornWebuiAutomationFacade::new(runtime.clone());
        let caller = caller();

        let output = facade
            .list_triggers(caller.clone(), Some(25))
            .await
            .expect("trigger list output");

        assert_eq!(output, json!({"trigger": {"ok": true}}));
        let request = runtime
            .requests
            .lock()
            .await
            .pop()
            .expect("runtime request");
        assert_eq!(request.capability_id.as_str(), TRIGGER_LIST_CAPABILITY_ID);
        assert_eq!(request.context.tenant_id, caller.tenant_id);
        assert_eq!(request.context.user_id, caller.user_id);
        assert_eq!(request.context.agent_id, caller.agent_id);
        assert_eq!(request.context.project_id, caller.project_id);
        assert_eq!(request.context.resource_scope.tenant_id, caller.tenant_id);
        assert_eq!(request.context.resource_scope.user_id, caller.user_id);
        assert_eq!(request.context.resource_scope.agent_id, caller.agent_id);
        assert_eq!(request.context.resource_scope.project_id, caller.project_id);
        assert_eq!(request.input["limit"], 25);
    }

    #[tokio::test]
    async fn automation_facade_redacts_runtime_failure_messages() {
        let runtime = Arc::new(FailingHostRuntime::new(RuntimeFailureKind::Internal));
        let facade = RebornWebuiAutomationFacade::new(runtime);
        let caller = caller();

        let error = facade
            .list_triggers(caller, Some(10))
            .await
            .expect_err("runtime failure should map to services error");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
        assert!(!format!("{error:?}").contains("redacted runtime details"));
    }

    fn caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new("tenant-alpha").expect("valid tenant"),
            UserId::new("user-alpha").expect("valid user"),
            Some(AgentId::new("agent-alpha").expect("valid agent")),
            Some(ProjectId::new("project-alpha").expect("valid project")),
        )
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
                    output: json!({"trigger": {"ok": true}}),
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
