use std::collections::HashMap;

use ironclaw_host_api::{http::RuntimeCredentialTarget, ids::SecretHandle};

use crate::{
    ProcessSandboxPlanError as SandboxPlanError, SandboxCommandPlan, SandboxCredentialBinding,
    SandboxInstallPlan, SandboxMounts, SandboxNetworkPlan, SandboxProcessPlan,
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
        RuntimeCredentialTarget::Basic {
            username: "api-user".to_string(),
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
