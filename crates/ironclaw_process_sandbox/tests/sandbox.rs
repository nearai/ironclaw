use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, MountView, ProcessId, ProjectId,
    ResourceEstimate, ResourceScope, RuntimeCredentialTarget, RuntimeKind, SecretHandle, TenantId,
    ThreadId, UserId,
};
use ironclaw_process_sandbox::{
    DEFAULT_PROCESS_SANDBOX_IMAGE, DockerBrokerConfig, DockerInvocation,
    DockerProcessSandboxBackend, DockerProcessSandboxConfig, DockerRunError, DockerRunOutput,
    DockerRunner, ProcessSandboxExecutor, SandboxBrokerPolicy, SandboxCommandPlan,
    SandboxCredentialBinding, SandboxInstallPlan, SandboxMounts, SandboxNetworkPlan,
    SandboxPlanError, SandboxProcessApprovalSummary, SandboxProcessPhase, SandboxProcessPlan,
    ValidatedSandboxProcessPlan, docker_invocation_for_phase,
};
use ironclaw_processes::{ProcessCancellationToken, ProcessExecutionRequest, ProcessExecutor};
use secrecy::SecretString;
use serde_json::Value;

fn sample_plan() -> SandboxProcessPlan {
    let mut env = HashMap::new();
    env.insert("NOTION_API_KEY".to_string(), "NOTION_API_KEY".to_string());
    SandboxProcessPlan {
        image: None,
        install: Some(SandboxInstallPlan {
            command: SandboxCommandPlan {
                command: "npm".to_string(),
                args: vec![
                    "install".to_string(),
                    "-g".to_string(),
                    "notion-cli".to_string(),
                ],
                env: HashMap::new(),
                working_dir: None,
                timeout_ms: None,
                max_stdout_bytes: None,
                max_stderr_bytes: None,
            },
            allowed_hosts: vec!["registry.npmjs.org".to_string()],
        }),
        run: SandboxCommandPlan {
            command: "notion".to_string(),
            args: vec!["list".to_string()],
            env,
            working_dir: Some("/workspace".to_string()),
            timeout_ms: Some(5_000),
            max_stdout_bytes: Some(4096),
            max_stderr_bytes: Some(4096),
        },
        mounts: SandboxMounts::default(),
        network: SandboxNetworkPlan {
            runtime_hosts: vec!["api.notion.com".to_string()],
            direct_egress_lockdown: true,
        },
        credentials: vec![SandboxCredentialBinding {
            handle: SecretHandle::new("notion_token").unwrap(),
            approved_host: "api.notion.com".to_string(),
            target: RuntimeCredentialTarget::Header {
                name: "Authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            placeholder_env: Some("NOTION_API_KEY".to_string()),
            placeholder_value: "NOTION_API_KEY".to_string(),
            required: true,
        }],
    }
}

fn sample_config(root: &Path) -> DockerProcessSandboxConfig {
    DockerProcessSandboxConfig {
        docker_bin: "docker".to_string(),
        image: DEFAULT_PROCESS_SANDBOX_IMAGE.to_string(),
        workspace_host_path: root.join("workspace"),
        tools_host_path: root.join("tools"),
        cache_host_path: root.join("cache"),
        broker: Some(DockerBrokerConfig {
            proxy_url: "http://host.docker.internal:4489".to_string(),
            ca_cert_host_path: root.join("broker-ca.pem"),
            ca_cert_container_path: "/ironclaw/broker/ca.pem".to_string(),
        }),
    }
}

fn validated_sample_plan() -> ValidatedSandboxProcessPlan {
    ValidatedSandboxProcessPlan::new(sample_plan()).unwrap()
}

#[test]
fn plan_validation_rejects_raw_secret_env_values() {
    let mut plan = sample_plan();
    plan.run.env.insert(
        "NOTION_API_KEY".to_string(),
        "real-secret-token".to_string(),
    );

    let error = plan.validate().unwrap_err();

    assert!(matches!(error, SandboxPlanError::RawSecretEnvValue { .. }));
}

#[test]
fn plan_validation_rejects_credential_host_missing_from_runtime_network() {
    let mut plan = sample_plan();
    plan.network.runtime_hosts.clear();

    let error = plan.validate().unwrap_err();

    assert!(matches!(
        error,
        SandboxPlanError::CredentialHostNotAllowed { .. }
            | SandboxPlanError::CredentialedRunWithoutRuntimeNetwork
    ));
}

#[test]
fn plan_validation_rejects_credentialed_run_without_lockdown() {
    let mut plan = sample_plan();
    plan.network.direct_egress_lockdown = false;

    let error = plan.validate().unwrap_err();

    assert_eq!(error, SandboxPlanError::CredentialedRunWithoutLockdown);
}

#[test]
fn plan_validation_rejects_writable_quarantine_during_credentialed_run() {
    let mut plan = sample_plan();
    plan.mounts.tools.writable = true;

    let error = plan.validate().unwrap_err();

    assert_eq!(error, SandboxPlanError::WritableStateDuringCredentialedRun);
}

#[test]
fn docker_args_never_include_secret_material() {
    let temp = tempfile::tempdir().unwrap();
    let plan = validated_sample_plan();
    let config = sample_config(temp.path());

    let invocation =
        docker_invocation_for_phase(&config, &plan, SandboxProcessPhase::Run, &plan.run).unwrap();
    let joined = invocation.args.join("\n");

    assert!(!joined.contains("real-notion-secret"));
    assert!(joined.contains("NOTION_API_KEY=NOTION_API_KEY"));
    assert!(joined.contains("IRONCLAW_EGRESS_LOCKDOWN=broker-only"));
    assert!(joined.contains("HTTP_PROXY=http://host.docker.internal:4489"));
}

#[test]
fn docker_builder_rejects_credentialed_run_without_broker() {
    let temp = tempfile::tempdir().unwrap();
    let plan = validated_sample_plan();
    let mut config = sample_config(temp.path());
    config.broker = None;

    let error = docker_invocation_for_phase(&config, &plan, SandboxProcessPhase::Run, &plan.run)
        .unwrap_err();

    assert_eq!(error, SandboxPlanError::CredentialedRunWithoutBroker);
}

#[test]
fn install_and_run_phases_have_different_mount_and_network_policies() {
    let temp = tempfile::tempdir().unwrap();
    let plan = validated_sample_plan();
    let config = sample_config(temp.path());
    let install = docker_invocation_for_phase(
        &config,
        &plan,
        SandboxProcessPhase::Install,
        &plan.install.as_ref().unwrap().command,
    )
    .unwrap();
    let run =
        docker_invocation_for_phase(&config, &plan, SandboxProcessPhase::Run, &plan.run).unwrap();
    let install_args = install.args.join("\n");
    let run_args = run.args.join("\n");

    assert!(install_args.contains("--network\nbridge"));
    assert!(!install_args.contains("IRONCLAW_EGRESS_LOCKDOWN=broker-only"));
    assert!(!install_args.contains("dst=/ironclaw/state/tools,readonly"));
    assert!(run_args.contains("IRONCLAW_EGRESS_LOCKDOWN=broker-only"));
    assert!(run_args.contains("dst=/ironclaw/state/tools,readonly"));
    assert!(run_args.contains("dst=/ironclaw/state/cache,readonly"));
}

#[test]
fn broker_policy_rewrites_only_approved_host_and_header() {
    let plan = sample_plan();
    let policy = SandboxBrokerPolicy::new(plan.credentials).unwrap();
    let mut secrets = HashMap::new();
    secrets.insert(
        SecretHandle::new("notion_token").unwrap(),
        SecretString::from("real-notion-secret"),
    );

    let approved = policy.rewrite_headers(
        "api.notion.com",
        vec![(
            "Authorization".to_string(),
            "Bearer NOTION_API_KEY".to_string(),
        )],
        &secrets,
    );
    let denied = policy.rewrite_headers(
        "example.com",
        vec![(
            "Authorization".to_string(),
            "Bearer NOTION_API_KEY".to_string(),
        )],
        &secrets,
    );

    assert_eq!(
        approved.headers,
        vec![(
            "Authorization".to_string(),
            "Bearer real-notion-secret".to_string()
        )]
    );
    assert_eq!(approved.rewrites.len(), 1);
    assert_eq!(
        denied.headers,
        vec![(
            "Authorization".to_string(),
            "Bearer NOTION_API_KEY".to_string()
        )]
    );
    assert!(denied.rewrites.is_empty());
}

#[test]
fn broker_redacts_secret_values_from_error_paths() {
    let plan = sample_plan();
    let policy = SandboxBrokerPolicy::new(plan.credentials).unwrap();
    let mut secrets = HashMap::new();
    secrets.insert(
        SecretHandle::new("notion_token").unwrap(),
        SecretString::from("real-notion-secret"),
    );

    let sanitized = policy.sanitize_text("upstream echoed real-notion-secret", &secrets);

    assert_eq!(sanitized, "upstream echoed [REDACTED]");
}

#[test]
fn approval_summary_contains_only_sanitized_authority_details() {
    let plan = sample_plan();

    let summary = SandboxProcessApprovalSummary::from_plan(&plan).unwrap();
    let serialized = serde_json::to_string(&summary).unwrap();

    assert_eq!(
        summary.install_command,
        Some(vec![
            "npm".to_string(),
            "install".to_string(),
            "-g".to_string(),
            "notion-cli".to_string()
        ])
    );
    assert_eq!(
        summary.run_command,
        vec!["notion".to_string(), "list".to_string()]
    );
    assert_eq!(summary.allowed_network_hosts, vec!["api.notion.com"]);
    assert!(summary.direct_egress_lockdown);
    assert_eq!(summary.credentials[0].secret_alias.as_str(), "notion_token");
    assert_eq!(
        summary.credentials[0].target,
        "header:Authorization=Bearer <secret>"
    );
    assert!(serialized.contains("NOTION_API_KEY"));
    assert!(!serialized.contains("real-notion-secret"));
}

#[derive(Default)]
struct RecordingRunner {
    invocations: Mutex<Vec<DockerInvocation>>,
}

#[async_trait]
impl DockerRunner for RecordingRunner {
    async fn run(
        &self,
        invocation: DockerInvocation,
        _command: &SandboxCommandPlan,
        _cancellation: ProcessCancellationToken,
    ) -> Result<DockerRunOutput, DockerRunError> {
        self.invocations.lock().unwrap().push(invocation);
        Ok(DockerRunOutput {
            exit_code: 0,
            stdout: b"{\"ok\":true}".to_vec(),
            stderr: Vec::new(),
            wall_clock_ms: 12,
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }
}

#[tokio::test]
async fn executor_returns_sanitized_phase_output_json() {
    let temp = tempfile::tempdir().unwrap();
    let runner = Arc::new(RecordingRunner::default());
    let backend = DockerProcessSandboxBackend::with_runner(
        sample_config(temp.path()),
        runner.clone() as Arc<dyn DockerRunner>,
    );
    let executor = ProcessSandboxExecutor::new(Arc::new(backend));

    let result = executor
        .execute(sample_request(serde_json::to_value(sample_plan()).unwrap()))
        .await
        .unwrap();

    assert_eq!(result.output["kind"], "process_sandbox_result");
    assert_eq!(result.output["phases"].as_array().unwrap().len(), 2);
    assert_eq!(runner.invocations.lock().unwrap().len(), 2);
}

struct CancellingRunner;

#[async_trait]
impl DockerRunner for CancellingRunner {
    async fn run(
        &self,
        _invocation: DockerInvocation,
        _command: &SandboxCommandPlan,
        cancellation: ProcessCancellationToken,
    ) -> Result<DockerRunOutput, DockerRunError> {
        cancellation.cancel();
        Err(DockerRunError::Cancelled)
    }
}

#[tokio::test]
async fn executor_surfaces_cancellation_as_stable_error_kind() {
    let temp = tempfile::tempdir().unwrap();
    let backend = DockerProcessSandboxBackend::with_runner(
        sample_config(temp.path()),
        Arc::new(CancellingRunner),
    );
    let executor = ProcessSandboxExecutor::new(Arc::new(backend));

    let error = executor
        .execute(sample_request(serde_json::to_value(sample_plan()).unwrap()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, "cancelled");
}

fn sample_request(input: Value) -> ProcessExecutionRequest {
    ProcessExecutionRequest {
        process_id: ProcessId::new(),
        invocation_id: InvocationId::new(),
        scope: ResourceScope {
            tenant_id: TenantId::new("tenant").unwrap(),
            user_id: UserId::new("user").unwrap(),
            agent_id: Some(AgentId::new("agent").unwrap()),
            project_id: Some(ProjectId::new("project").unwrap()),
            mission_id: None,
            thread_id: Some(ThreadId::new("thread").unwrap()),
            invocation_id: InvocationId::new(),
        },
        extension_id: ExtensionId::new("system.process_sandbox").unwrap(),
        capability_id: CapabilityId::new("system.process_sandbox.run").unwrap(),
        runtime: RuntimeKind::System,
        estimate: ResourceEstimate::default(),
        mounts: MountView::default(),
        resource_reservation: None,
        input,
        cancellation: ProcessCancellationToken::new(),
    }
}
