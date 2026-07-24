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

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::ProductSurfaceCaller;
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
use ironclaw_product::{
    AUTOMATIONS_VIEW, ProductListAutomationsRequest, RebornListAutomationsResponse,
};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornHostBindings, RebornRuntimeIdentity, RebornRuntimeInput,
    RebornRuntimeProcessBinding, build_reborn_runtime,
};

#[path = "support/first_party.rs"]
mod first_party_support;

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

    let input = RebornRuntimeInput::from_build_input(
        RebornHostBindings::libsql(
            RebornCompositionProfile::Production,
            "runtime-automation-prod-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_first_party_bundles(first_party_support::test_first_party_bundles())
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

    let bundle = runtime
        .product_surface(None)
        .expect("product surface builds");
    let caller = ProductSurfaceCaller::new(
        TenantId::new("runtime-automation-prod-tenant").unwrap(),
        UserId::new("runtime-automation-prod-owner").unwrap(),
        Some(AgentId::new("runtime-automation-prod-agent").unwrap()),
        None,
    );

    // An empty list is fine — the key invariant is that the facade is wired
    // (no 503) so the request reaches the repository rather than returning
    // ServiceUnavailable.
    let result = ironclaw_host_api::ProductSurface::query(
        bundle.as_ref(),
        caller,
        ironclaw_host_api::ProductSurfaceQueryRequest {
            view_id: AUTOMATIONS_VIEW.id.to_string(),
            input: serde_json::to_value(ProductListAutomationsRequest::default())
                .expect("automation list params"),
            cursor: None,
            limit: None,
        },
    )
    .await
    .expect("production automation facade must be reachable (not 503)");
    let result = ironclaw_product::RebornViewPage {
        payload: result
            .items
            .into_iter()
            .next()
            .expect("automation list payload"),
        next_cursor: result.next_cursor,
    };
    let result: RebornListAutomationsResponse =
        serde_json::from_value(result.payload).expect("automation list response");
    assert_eq!(
        result.automations.len(),
        0,
        "empty repository returns zero automations"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}
