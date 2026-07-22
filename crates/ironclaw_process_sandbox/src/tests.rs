use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, MountView, ProcessId, ProjectId,
    ResourceEstimate, ResourceScope, RuntimeCredentialTarget, RuntimeKind, SecretHandle, TenantId,
    ThreadId, UserId,
};
use ironclaw_processes::{ProcessCancellationToken, ProcessExecutionRequest, ProcessExecutor};
use secrecy::SecretString;
use serde_json::Value;

use crate::{
    BrokerRewriteError, DEFAULT_PROCESS_SANDBOX_IMAGE, DockerBrokerConfig,
    DockerProcessSandboxBackend, DockerProcessSandboxConfig, ProcessSandboxBackend,
    ProcessSandboxError, ProcessSandboxExecutor, ProcessSandboxPlanError as SandboxPlanError,
    SandboxBrokerPolicy, SandboxCommandPlan, SandboxCredentialBinding, SandboxInstallPlan,
    SandboxMounts, SandboxNetworkPlan, SandboxProcessApprovalSummary, SandboxProcessOutput,
    SandboxProcessPhase, SandboxProcessPlan, SandboxProcessRequest, SandboxProcessResult,
    ValidatedSandboxProcessPlan,
    docker::{
        DockerInvocation, DockerRunError, DockerRunOutput, DockerRunner, broker_host,
        docker_invocation_for_phase,
    },
    validation::{is_container_absolute_path, validate_header_name, validate_host},
};

fn sample_plan() -> SandboxProcessPlan {
    let mut env = HashMap::new();
    env.insert("NOTION_API_KEY".to_string(), "NOTION_API_KEY".to_string());
    SandboxProcessPlan {
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
            allowed_hosts: Vec::new(),
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
fn plan_validation_rejects_unsupported_credential_targets() {
    for target in [
        RuntimeCredentialTarget::QueryParam {
            name: "access_token".to_string(),
        },
        RuntimeCredentialTarget::PathPlaceholder {
            placeholder: "__credential__".to_string(),
        },
    ] {
        let mut plan = sample_plan();
        plan.credentials[0].target = target;

        let error = plan.validate().unwrap_err();

        assert_eq!(error, SandboxPlanError::UnsupportedCredentialTarget);
    }
}

#[test]
fn plan_validation_rejects_writable_quarantine_during_credentialed_run() {
    let mut plan = sample_plan();
    plan.mounts.tools.writable = true;

    let error = plan.validate().unwrap_err();

    assert_eq!(error, SandboxPlanError::WritableStateDuringCredentialedRun);
}

#[test]
fn plan_validation_rejects_unbounded_runtime_limits() {
    let mut plan = sample_plan();
    plan.run.timeout_ms = Some(crate::MAX_TIMEOUT_MS + 1);

    let timeout_error = plan.validate().unwrap_err();

    let mut plan = sample_plan();
    plan.run.max_stdout_bytes = Some(crate::MAX_OUTPUT_LIMIT + 1);
    let stdout_error = plan.validate().unwrap_err();

    assert!(matches!(
        timeout_error,
        SandboxPlanError::TimeoutLimitTooLarge { phase: "run", .. }
    ));
    assert!(matches!(
        stdout_error,
        SandboxPlanError::OutputLimitTooLarge {
            phase: "run",
            stream: "stdout",
            ..
        }
    ));
}

#[test]
fn plan_validation_rejects_mounts_over_system_paths() {
    let mut plan = sample_plan();
    plan.mounts.workspace.container_path = "/etc/ironclaw".to_string();

    let error = plan.validate().unwrap_err();

    assert_eq!(
        error,
        SandboxPlanError::InvalidContainerPath {
            path: "/etc/ironclaw".to_string()
        }
    );
}

#[test]
fn plan_validation_rejects_mount_paths_that_break_mount_specs() {
    let mut plan = sample_plan();
    plan.mounts.workspace.container_path = "/workspace,src=/etc".to_string();

    let error = plan.validate().unwrap_err();

    assert_eq!(
        error,
        SandboxPlanError::InvalidContainerPath {
            path: "/workspace,src=/etc".to_string()
        }
    );
}

#[test]
fn plan_validation_rejects_entrypoint_control_env_names() {
    let mut plan = sample_plan();
    plan.run
        .env
        .insert("LD_PRELOAD".to_string(), "x".to_string());

    let error = plan.validate().unwrap_err();

    assert_eq!(
        error,
        SandboxPlanError::InvalidEnvName {
            env: "LD_PRELOAD".to_string()
        }
    );
}

#[test]
fn plan_validation_rejects_malformed_command_fields() {
    let cases = [
        (
            "empty command",
            {
                let mut plan = sample_plan();
                plan.run.command.clear();
                plan
            },
            SandboxPlanError::EmptyCommand { phase: "run" },
        ),
        (
            "flag command",
            {
                let mut plan = sample_plan();
                plan.run.command = "--help".to_string();
                plan
            },
            SandboxPlanError::UnsafeCommand { phase: "run" },
        ),
        (
            "shell words",
            {
                let mut plan = sample_plan();
                plan.run.command = "notion cli".to_string();
                plan
            },
            SandboxPlanError::UnsafeCommand { phase: "run" },
        ),
        (
            "relative working directory",
            {
                let mut plan = sample_plan();
                plan.run.working_dir = Some("workspace".to_string());
                plan
            },
            SandboxPlanError::InvalidContainerPath {
                path: "workspace".to_string(),
            },
        ),
        (
            "invalid env name",
            {
                let mut plan = sample_plan();
                plan.run
                    .env
                    .insert("lowercase".to_string(), "1".to_string());
                plan
            },
            SandboxPlanError::InvalidEnvName {
                env: "lowercase".to_string(),
            },
        ),
        (
            "nul env value",
            {
                let mut plan = sample_plan();
                plan.run
                    .env
                    .insert("SAFE_ENV".to_string(), "a\0b".to_string());
                plan
            },
            SandboxPlanError::InvalidEnvValue {
                env: "SAFE_ENV".to_string(),
            },
        ),
    ];

    for (name, plan, expected) in cases {
        let error = plan.validate().unwrap_err();
        assert_eq!(error, expected, "{name}");
    }
}

#[test]
fn plan_validation_does_not_reject_sensitive_env_substrings_inside_words() {
    let mut plan = sample_plan();
    plan.run.env.clear();
    plan.credentials.clear();
    plan.network.runtime_hosts.clear();
    plan.network.direct_egress_lockdown = false;
    plan.run
        .env
        .insert("AUTHOR".to_string(), "alice".to_string());
    plan.run.env.insert(
        "TOKENIZER_PATH".to_string(),
        "/models/tokenizer".to_string(),
    );

    plan.validate().unwrap();
}

#[test]
fn plan_validation_rejects_common_sensitive_env_names() {
    for env_name in [
        "PRIVATE_KEY",
        "SERVICE_CREDENTIAL",
        "SIGNING_KEY",
        "ENCRYPTION_KEY",
        "SYMMETRIC_KEY",
        "BEARER_TOKEN",
    ] {
        let mut plan = sample_plan();
        plan.run.env.clear();
        plan.credentials.clear();
        plan.network.runtime_hosts.clear();
        plan.network.direct_egress_lockdown = false;
        plan.run
            .env
            .insert(env_name.to_string(), "raw-secret".to_string());

        let error = plan.validate().unwrap_err();

        assert!(matches!(error, SandboxPlanError::RawSecretEnvValue { .. }));
    }
}

#[test]
fn validation_rejects_invalid_hosts() {
    for host in [
        "",
        "https://api.notion.com",
        "api.notion.com:443",
        "api notion",
    ] {
        let error = validate_host(host).unwrap_err();

        assert!(matches!(error, SandboxPlanError::InvalidHost { .. }));
    }
}

#[test]
fn validation_rejects_invalid_header_names() {
    for header in ["", "Authorization Token", "Bad:Header", "Bad(Header)"] {
        let error = validate_header_name(header).unwrap_err();

        assert_eq!(error, SandboxPlanError::InvalidCredentialTarget);
    }
}

#[test]
fn validation_rejects_invalid_container_paths() {
    for path in ["/workspace\0x", "/workspace,src=/etc", "/workspace/../etc"] {
        assert!(!is_container_absolute_path(path), "{path}");
    }
}

#[test]
fn plan_validation_rejects_duplicate_credential_targets() {
    let mut plan = sample_plan();
    let mut duplicate = plan.credentials[0].clone();
    duplicate.approved_host = "API.NOTION.COM".to_string();
    duplicate.target = RuntimeCredentialTarget::Header {
        name: "authorization".to_string(),
        prefix: Some("Bearer ".to_string()),
    };
    plan.credentials.push(duplicate);

    let error = plan.validate().unwrap_err();

    assert!(matches!(
        error,
        SandboxPlanError::DuplicateCredentialTarget { .. }
    ));
}

#[test]
fn plan_validation_rejects_missing_or_mismatched_placeholder_env() {
    let mut missing = sample_plan();
    missing.run.env.clear();
    let missing_error = missing.validate().unwrap_err();

    let mut mismatched = sample_plan();
    mismatched.run.env.clear();
    mismatched.credentials[0].placeholder_env = Some("PLACEHOLDER".to_string());
    mismatched.credentials[0].placeholder_value = "PLACEHOLDER".to_string();
    mismatched
        .run
        .env
        .insert("PLACEHOLDER".to_string(), "WRONG_PLACEHOLDER".to_string());
    let mismatched_error = mismatched.validate().unwrap_err();

    assert_eq!(
        missing_error,
        SandboxPlanError::MissingPlaceholderEnv {
            env: "NOTION_API_KEY".to_string()
        }
    );
    assert_eq!(
        mismatched_error,
        SandboxPlanError::InvalidPlaceholderEnv {
            env: "PLACEHOLDER".to_string()
        }
    );
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
    assert!(joined.contains("HTTPS_PROXY=http://host.docker.internal:4489"));
    assert!(joined.contains("http_proxy=http://host.docker.internal:4489"));
    assert!(joined.contains("https_proxy=http://host.docker.internal:4489"));
    assert!(joined.contains("--add-host\nhost.docker.internal:host-gateway"));
    assert!(joined.contains("--memory\n512m"));
    assert!(joined.contains("--pids-limit\n256"));
    assert!(joined.contains("--cap-add\nSETUID"));
    assert!(joined.contains(DEFAULT_PROCESS_SANDBOX_IMAGE));
    assert!(
        invocation
            .container_name
            .starts_with("ironclaw-sandbox-run-")
    );
}

#[test]
fn docker_broker_host_parses_proxy_url_hosts() {
    let host_gateway = broker_host("http://host.docker.internal:4489");
    let path_host = broker_host("https://broker.local/path");

    assert_eq!(host_gateway, Some("host.docker.internal"));
    assert_eq!(path_host, Some("broker.local"));
    assert_eq!(broker_host("broker.local:4489"), None);
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

    assert!(install_args.contains("--network\nnone"));
    assert!(!install_args.contains("IRONCLAW_EGRESS_LOCKDOWN=broker-only"));
    assert!(install_args.contains("dst=/ironclaw/state/tools,readonly"));
    assert!(install_args.contains("dst=/ironclaw/state/cache,readonly"));
    assert!(run_args.contains("IRONCLAW_EGRESS_LOCKDOWN=broker-only"));
    assert!(run_args.contains("dst=/ironclaw/state/tools,readonly"));
    assert!(run_args.contains("dst=/ironclaw/state/cache,readonly"));
}

#[test]
fn docker_invocation_rejects_unenforced_network_hosts() {
    let temp = tempfile::tempdir().unwrap();
    let mut plan = sample_plan();
    plan.run.env.clear();
    plan.network.direct_egress_lockdown = false;
    plan.credentials.clear();
    let plan = ValidatedSandboxProcessPlan::new(plan).unwrap();
    let config = sample_config(temp.path());

    let error = docker_invocation_for_phase(&config, &plan, SandboxProcessPhase::Run, &plan.run)
        .unwrap_err();

    assert_eq!(
        error,
        SandboxPlanError::UnenforcedNetworkHosts { phase: "run" }
    );
}

#[test]
fn docker_invocation_rejects_unenforced_install_allowed_hosts() {
    let temp = tempfile::tempdir().unwrap();
    let mut plan = sample_plan();
    plan.install.as_mut().unwrap().allowed_hosts = vec!["registry.npmjs.org".to_string()];
    let plan = ValidatedSandboxProcessPlan::new(plan).unwrap();
    let config = sample_config(temp.path());

    let error = docker_invocation_for_phase(
        &config,
        &plan,
        SandboxProcessPhase::Install,
        &plan.install.as_ref().unwrap().command,
    )
    .unwrap_err();

    assert_eq!(
        error,
        SandboxPlanError::UnenforcedNetworkHosts { phase: "install" }
    );
}

#[test]
fn docker_invocation_rejects_host_paths_that_break_mount_specs() {
    let temp = tempfile::tempdir().unwrap();
    let plan = validated_sample_plan();
    let mut config = sample_config(temp.path());
    config.workspace_host_path = temp.path().join("workspace,with-comma");

    let error = docker_invocation_for_phase(&config, &plan, SandboxProcessPhase::Run, &plan.run)
        .unwrap_err();

    assert!(matches!(error, SandboxPlanError::InvalidHostPath { .. }));
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

    let approved = policy
        .rewrite_headers(
            "api.notion.com:443",
            vec![(
                "Authorization".to_string(),
                "Bearer NOTION_API_KEY".to_string(),
            )],
            &secrets,
        )
        .unwrap();
    let denied = policy
        .rewrite_headers(
            "example.com",
            vec![(
                "Authorization".to_string(),
                "Bearer NOTION_API_KEY".to_string(),
            )],
            &secrets,
        )
        .unwrap();

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
fn broker_policy_rejects_missing_required_secret() {
    let plan = sample_plan();
    let policy = SandboxBrokerPolicy::new(plan.credentials).unwrap();
    let error = policy
        .rewrite_headers(
            "api.notion.com",
            vec![(
                "Authorization".to_string(),
                "Bearer NOTION_API_KEY".to_string(),
            )],
            &HashMap::new(),
        )
        .unwrap_err();

    assert_eq!(
        error,
        BrokerRewriteError::MissingRequiredSecret {
            secret_alias: SecretHandle::new("notion_token").unwrap()
        }
    );
}

#[test]
fn broker_policy_rejects_duplicate_credential_targets() {
    let mut plan = sample_plan();
    let mut duplicate = plan.credentials[0].clone();
    duplicate.approved_host = "API.NOTION.COM".to_string();
    duplicate.target = RuntimeCredentialTarget::Header {
        name: "authorization".to_string(),
        prefix: Some("Bearer ".to_string()),
    };
    plan.credentials.push(duplicate);

    let error = SandboxBrokerPolicy::new(plan.credentials).unwrap_err();

    assert!(matches!(
        error,
        SandboxPlanError::DuplicateCredentialTarget { .. }
    ));
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
fn broker_redacts_longer_secret_before_embedded_substring() {
    let policy = SandboxBrokerPolicy::new(Vec::new()).unwrap();
    let mut secrets = HashMap::new();
    secrets.insert(
        SecretHandle::new("short").unwrap(),
        SecretString::from("token"),
    );
    secrets.insert(
        SecretHandle::new("long").unwrap(),
        SecretString::from("token-extended"),
    );

    let sanitized = policy.sanitize_text("upstream echoed token-extended", &secrets);

    assert_eq!(sanitized, "upstream echoed [REDACTED]");
}

#[test]
fn broker_policy_allows_empty_bindings() {
    let policy = SandboxBrokerPolicy::new(Vec::new()).unwrap();
    let result = policy
        .rewrite_headers(
            "api.notion.com",
            vec![(
                "Authorization".to_string(),
                "Bearer placeholder".to_string(),
            )],
            &HashMap::new(),
        )
        .unwrap();

    let expected_headers = vec![(
        "Authorization".to_string(),
        "Bearer placeholder".to_string(),
    )];
    assert_eq!(result.headers, expected_headers);
    assert!(result.rewrites.is_empty());
}

#[test]
fn approval_summary_contains_only_sanitized_authority_details() {
    let mut plan = sample_plan();
    plan.install.as_mut().unwrap().allowed_hosts = vec!["registry.npmjs.org".to_string()];

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
    assert_eq!(summary.install_allowed_hosts, vec!["registry.npmjs.org"]);
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

#[derive(Default)]
struct InstallFailsRunner {
    calls: AtomicUsize,
}

#[async_trait]
impl DockerRunner for InstallFailsRunner {
    async fn run(
        &self,
        invocation: DockerInvocation,
        _command: &SandboxCommandPlan,
        _cancellation: ProcessCancellationToken,
    ) -> Result<DockerRunOutput, DockerRunError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(DockerRunOutput {
            exit_code: if invocation.phase == SandboxProcessPhase::Install && call == 0 {
                42
            } else {
                0
            },
            stdout: b"install failed".to_vec(),
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

#[tokio::test]
async fn docker_backend_stops_after_failed_install_phase() {
    let temp = tempfile::tempdir().unwrap();
    let runner = Arc::new(InstallFailsRunner::default());
    let backend = DockerProcessSandboxBackend::with_runner(
        sample_config(temp.path()),
        runner.clone() as Arc<dyn DockerRunner>,
    );

    let result = backend
        .execute(SandboxProcessRequest {
            process_id: ProcessId::new(),
            scope: sample_request(serde_json::json!({})).scope,
            plan: validated_sample_plan(),
            cancellation: ProcessCancellationToken::new(),
        })
        .await
        .unwrap();

    assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.output.phases.len(), 1);
    assert_eq!(result.output.phases[0].phase, SandboxProcessPhase::Install);
    assert_eq!(result.output.phases[0].exit_code, 42);
}

#[derive(Clone)]
struct FailingRunner {
    error: DockerRunError,
}

#[async_trait]
impl DockerRunner for FailingRunner {
    async fn run(
        &self,
        _invocation: DockerInvocation,
        _command: &SandboxCommandPlan,
        cancellation: ProcessCancellationToken,
    ) -> Result<DockerRunOutput, DockerRunError> {
        if self.error == DockerRunError::Cancelled {
            cancellation.cancel();
        }
        Err(self.error.clone())
    }
}

#[tokio::test]
async fn executor_maps_docker_runner_errors_to_stable_kinds() {
    for (runner_error, expected_kind) in [
        (DockerRunError::Spawn, "docker_spawn_failed"),
        (DockerRunError::Io, "docker_io_failed"),
        (DockerRunError::Cancelled, "cancelled"),
        (DockerRunError::Timeout, "timeout"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let backend = DockerProcessSandboxBackend::with_runner(
            sample_config(temp.path()),
            Arc::new(FailingRunner {
                error: runner_error,
            }),
        );
        let executor = ProcessSandboxExecutor::new(Arc::new(backend));

        let error = executor
            .execute(sample_request(serde_json::to_value(sample_plan()).unwrap()))
            .await
            .unwrap_err();

        assert_eq!(error.kind, expected_kind);
    }
}

#[derive(Default)]
struct CountingBackend {
    calls: AtomicUsize,
}

#[async_trait]
impl ProcessSandboxBackend for CountingBackend {
    async fn execute(
        &self,
        _request: SandboxProcessRequest,
    ) -> Result<SandboxProcessResult, ProcessSandboxError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(SandboxProcessResult {
            output: SandboxProcessOutput::default(),
        })
    }
}

#[tokio::test]
async fn executor_rejects_invalid_process_sandbox_plan_without_backend_execution() {
    let backend = Arc::new(CountingBackend::default());
    let executor = ProcessSandboxExecutor::new(backend.clone());

    let malformed = executor
        .execute(sample_request(serde_json::json!({ "run": 1 })))
        .await
        .unwrap_err();
    let invalid = executor
        .execute(sample_request(
            serde_json::to_value({
                let mut plan = sample_plan();
                plan.run.command.clear();
                plan
            })
            .unwrap(),
        ))
        .await
        .unwrap_err();

    assert_eq!(malformed.kind, "invalid_process_sandbox_plan");
    assert_eq!(invalid.kind, "invalid_process_sandbox_plan");
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
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
        authenticated_actor_user_id: None,
        extension_id: ExtensionId::new("system.process_sandbox").unwrap(),
        capability_id: CapabilityId::new("system.process_sandbox.run").unwrap(),
        runtime: RuntimeKind::System,
        estimate: ResourceEstimate::default(),
        mounts: MountView::default(),
        resource_reservation: None,
        authorized_continuation: None,
        input,
        cancellation: ProcessCancellationToken::new(),
    }
}
