use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use ironclaw_reborn_composition::{
    RebornBuildInput, RebornRuntimeProcessBinding, build_reborn_services,
};

#[tokio::test]
async fn local_dev_composes_injected_tenant_sandbox_process_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transport = Arc::new(RecordingSandboxTransport::default());
    let process_port = Arc::new(ironclaw_host_runtime::TenantSandboxProcessPort::new(
        transport.clone(),
    ));
    let services = build_reborn_services(
        RebornBuildInput::local_dev("sandbox-port-owner", dir.path().join("local-dev"))
            .with_runtime_policy(tenant_sandbox_process_policy())
            .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(
                process_port,
            )),
    )
    .await
    .expect("local-dev services build");
    let host_runtime = services.host_runtime.expect("host runtime");

    let output = invoke_shell(
        host_runtime.as_ref(),
        serde_json::json!({"command": "echo composed sandbox", "timeout": 9}),
    )
    .await;

    assert_eq!(output["sandboxed"], serde_json::json!(true));
    assert_eq!(
        output["output"],
        serde_json::json!("sandbox port: echo composed sandbox")
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].command, "echo composed sandbox");
    assert_eq!(requests[0].timeout_secs, Some(9));
}

#[derive(Debug, Default)]
struct RecordingSandboxTransport {
    requests: Mutex<Vec<ironclaw_host_runtime::CommandExecutionRequest>>,
}

#[async_trait::async_trait]
impl ironclaw_host_runtime::SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        request: ironclaw_host_runtime::CommandExecutionRequest,
    ) -> Result<
        ironclaw_host_runtime::CommandExecutionOutput,
        ironclaw_host_runtime::RuntimeProcessError,
    > {
        let command = request.command.clone();
        self.requests.lock().unwrap().push(request);
        Ok(ironclaw_host_runtime::CommandExecutionOutput {
            output: format!("sandbox port: {command}"),
            saved_output: None,
            exit_code: 0,
            sandboxed: false,
            duration: Duration::from_millis(5),
        })
    }
}

async fn invoke_shell(
    runtime: &dyn ironclaw_host_runtime::HostRuntime,
    input: serde_json::Value,
) -> serde_json::Value {
    let outcome = runtime
        .invoke_capability(ironclaw_host_runtime::RuntimeCapabilityRequest::new(
            shell_execution_context(),
            ironclaw_host_api::CapabilityId::new(ironclaw_host_runtime::SHELL_CAPABILITY_ID)
                .unwrap(),
            ironclaw_host_api::ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .expect("capability invoke");
    let ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed shell invocation, got {outcome:?}");
    };
    completed.output
}

fn tenant_sandbox_process_policy() -> ironclaw_host_api::EffectiveRuntimePolicy {
    ironclaw_host_api::EffectiveRuntimePolicy {
        deployment: ironclaw_host_api::DeploymentMode::LocalSingleUser,
        requested_profile: ironclaw_host_api::RuntimeProfile::LocalDev,
        resolved_profile: ironclaw_host_api::RuntimeProfile::LocalDev,
        filesystem_backend: ironclaw_host_api::FilesystemBackendKind::HostWorkspace,
        process_backend: ironclaw_host_api::ProcessBackendKind::TenantSandbox,
        network_mode: ironclaw_host_api::NetworkMode::DirectLogged,
        secret_mode: ironclaw_host_api::SecretMode::ScrubbedEnv,
        approval_policy: ironclaw_host_api::ApprovalPolicy::AskDestructive,
        audit_mode: ironclaw_host_api::AuditMode::LocalMinimal,
    }
}

fn shell_execution_context() -> ironclaw_host_api::ExecutionContext {
    let grant = ironclaw_host_api::CapabilityGrant {
        id: ironclaw_host_api::CapabilityGrantId::new(),
        capability: ironclaw_host_api::CapabilityId::new(
            ironclaw_host_runtime::SHELL_CAPABILITY_ID,
        )
        .unwrap(),
        grantee: ironclaw_host_api::Principal::Extension(
            ironclaw_host_api::ExtensionId::new("caller").unwrap(),
        ),
        issued_by: ironclaw_host_api::Principal::Extension(
            ironclaw_host_api::ExtensionId::new("issuer").unwrap(),
        ),
        constraints: ironclaw_host_api::GrantConstraints {
            allowed_effects: shell_effects(),
            mounts: ironclaw_host_api::MountView::default(),
            network: shell_test_policy(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    };
    ironclaw_host_api::ExecutionContext::local_default(
        ironclaw_host_api::UserId::new("user").unwrap(),
        ironclaw_host_api::ExtensionId::new("caller").unwrap(),
        ironclaw_host_api::RuntimeKind::FirstParty,
        ironclaw_host_api::TrustClass::FirstParty,
        ironclaw_host_api::CapabilitySet {
            grants: vec![grant],
        },
        ironclaw_host_api::MountView::default(),
    )
    .unwrap()
}

fn shell_effects() -> Vec<ironclaw_host_api::EffectKind> {
    vec![
        ironclaw_host_api::EffectKind::DispatchCapability,
        ironclaw_host_api::EffectKind::ReadFilesystem,
        ironclaw_host_api::EffectKind::WriteFilesystem,
        ironclaw_host_api::EffectKind::Network,
        ironclaw_host_api::EffectKind::SpawnProcess,
        ironclaw_host_api::EffectKind::ExecuteCode,
    ]
}

fn shell_test_policy() -> ironclaw_host_api::NetworkPolicy {
    ironclaw_host_api::NetworkPolicy {
        allowed_targets: vec![ironclaw_host_api::NetworkTargetPattern {
            scheme: None,
            host_pattern: "*".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: None,
    }
}

fn trust_decision() -> ironclaw_trust::TrustDecision {
    ironclaw_trust::TrustDecision {
        effective_trust: ironclaw_trust::EffectiveTrustClass::user_trusted(),
        authority_ceiling: ironclaw_trust::AuthorityCeiling {
            allowed_effects: shell_effects(),
            max_resource_ceiling: None,
        },
        provenance: ironclaw_trust::TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}
