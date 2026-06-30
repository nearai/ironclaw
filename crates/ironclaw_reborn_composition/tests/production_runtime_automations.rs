//! Integration test that fully builds a production-shape `RebornRuntime` and
//! asserts the automation facade is reachable (no 503 `ServiceUnavailable`)
//! when the production profile is used.
//!
//! This test lives in its own integration-test binary so cargo executes it
//! sequentially with respect to lib unit tests.  The lib unit tests include
//! `local_dev_runtime_*` tests with hard 3-second `RunTimeout` budgets; if
//! this production-runtime build runs in the same binary it starves those
//! tests on parallel CPU-heavy builds.
//!
//! Gated on `libsql` because the production-runtime path under test requires
//! the libsql substrate.
#![cfg(feature = "libsql")]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, TenantId, UserId,
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
use ironclaw_host_runtime::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
    TenantSandboxProcessPort,
};
use ironclaw_product_workflow::{WebUiAuthenticatedCaller, WebUiListAutomationsRequest};
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornCompositionProfile, RebornRuntimeIdentity, RebornRuntimeInput,
    RebornRuntimeProcessBinding, build_reborn_runtime, build_webui_services,
    builtin_first_party_trust_policy,
};

// ─── minimal sandbox transport stub ──────────────────────────────────────────

#[derive(Debug)]
struct RecordingSandboxTransport;

#[async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::ZERO,
        })
    }
}

// ─── test ─────────────────────────────────────────────────────────────────────

/// Regression guard: production profiles have `local_runtime: None` and
/// `production_runtime: Some(...)`. The automation facade must be installed
/// from the production store graph so that `/automations` returns results
/// instead of a 503 ServiceUnavailable error.
#[tokio::test]
async fn production_runtime_webui_serves_automations_without_local_runtime() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db"))
            .build()
            .await
            .expect("libsql db"),
    );

    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "runtime-automation-prod-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_production_trust_policy(Arc::new(
            builtin_first_party_trust_policy().expect("trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::SecureDefault,
            resolved_profile: RuntimeProfile::SecureDefault,
            filesystem_backend: FilesystemBackendKind::ScopedVirtual,
            process_backend: ProcessBackendKind::TenantSandbox,
            network_mode: NetworkMode::Deny,
            secret_mode: SecretMode::BrokeredHandles,
            approval_policy: ApprovalPolicy::AskAlways,
            audit_mode: AuditMode::Standard,
        })
        .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
            TenantSandboxProcessPort::new(Arc::new(RecordingSandboxTransport)),
        ))),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-automation-prod-tenant".to_string(),
        agent_id: "runtime-automation-prod-agent".to_string(),
        source_binding_id: "runtime-automation-prod-source".to_string(),
        reply_target_binding_id: "runtime-automation-prod-reply".to_string(),
    });

    let runtime = build_reborn_runtime(input)
        .await
        .expect("production runtime builds");

    let bundle = build_webui_services(&runtime, None).expect("webui bundle builds");
    let caller = WebUiAuthenticatedCaller::new(
        TenantId::new("runtime-automation-prod-tenant").unwrap(),
        UserId::new("runtime-automation-prod-owner").unwrap(),
        Some(AgentId::new("runtime-automation-prod-agent").unwrap()),
        None,
    );

    // An empty list is fine — the key invariant is that the facade is wired
    // (no 503) so the request reaches the repository rather than returning
    // ServiceUnavailable.
    let result = bundle
        .api
        .list_automations(caller, WebUiListAutomationsRequest::default())
        .await
        .expect("production automation facade must be reachable (not 503)");
    assert_eq!(
        result.automations.len(),
        0,
        "empty repository returns zero automations"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}
