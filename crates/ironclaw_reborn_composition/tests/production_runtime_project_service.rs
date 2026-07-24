//! Integration test: a production-shaped `RebornRuntime` wires the first-class
//! projects (ACL) facade over its production store graph, and that facade is
//! tenant/user scoped.
//!
//! Regression for the bucket-2 production-parity gap (#5013 / audit #6389).
//! Production profiles have `local_runtime: None`; `runtime.product_surface` only
//! called `with_project_service` for the local substrate, so on production the
//! WebUI project surface fell through to the `ProductSurface` default, which
//! returns `service_unavailable` for `create_project` / `list_projects`. The
//! provisioner-style production fallback now sources the project service from
//! the production store graph (`RebornProjectService` over
//! `FilesystemProjectRepository` on the production scoped filesystem).
//!
//! Lives in its own integration-test binary (mirroring
//! `production_runtime_automations.rs`) so the CPU-heavy production build does
//! not starve the lib unit tests' hard `RunTimeout` budgets, and is gated on
//! `libsql` because the production-runtime path requires the libSQL substrate.

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
    PROJECT_CREATE_COMMAND, PROJECTS_VIEW, RebornCreateProjectRequest, RebornListProjectsRequest,
};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornHostBindings, RebornRuntimeIdentity, RebornRuntimeInput,
    RebornRuntimeProcessBinding, build_reborn_runtime,
};

#[path = "support/first_party.rs"]
mod first_party_support;

const RUNTIME_TENANT: &str = "prod-projects-tenant";
const RUNTIME_AGENT: &str = "prod-projects-agent";
const OWNER: &str = "prod-projects-owner";

// ─── minimal sandbox transport stub (production requires a process binding) ───

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

fn create_request(name: &str) -> RebornCreateProjectRequest {
    RebornCreateProjectRequest {
        name: name.to_string(),
        description: "bucket-2 production parity".to_string(),
        icon: None,
        color: None,
        metadata: None,
    }
}

/// Regression guard: on a production runtime the project facade is reachable
/// (not `service_unavailable`), persists round-trip over the production
/// substrate, and is scoped so a different tenant cannot see the project.
#[tokio::test]
async fn production_runtime_wires_project_service_and_scopes_by_tenant() {
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
            OWNER,
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
        tenant_id: RUNTIME_TENANT.to_string(),
        agent_id: RUNTIME_AGENT.to_string(),
        source_binding_id: "prod-projects-source".to_string(),
        reply_target_binding_id: "prod-projects-reply".to_string(),
    });

    let runtime = build_reborn_runtime(input)
        .await
        .expect("production runtime builds");
    let bundle = runtime
        .product_surface(None)
        .expect("product surface builds");

    let owner = ProductSurfaceCaller::new(
        TenantId::new(RUNTIME_TENANT).unwrap(),
        UserId::new(OWNER).unwrap(),
        Some(AgentId::new(RUNTIME_AGENT).unwrap()),
        None,
    );

    // (1) THE WIRING. Before the production fallback, the facade fell through to
    // the `ProductSurface` default and this returned
    // `service_unavailable`. A successful create proves `with_project_service`
    // was wired from the production store graph.
    let owner_surface = ironclaw_host_api::BoundProductSurface::new(bundle.clone(), owner.clone());
    let created = PROJECT_CREATE_COMMAND
        .invoke_on(
            &owner_surface,
            create_request("Prod Project"),
            ironclaw_host_api::ActivityId::new(),
        )
        .await
        .expect("production project facade must be reachable (not service_unavailable)");
    assert_eq!(created.project.name, "Prod Project");
    let project_id = created.project.project_id.clone();

    // (2) ROUND-TRIP over the production scoped filesystem: the created project
    // lists back for its owner.
    let listed = PROJECTS_VIEW
        .query_on(
            &owner_surface,
            RebornListProjectsRequest { limit: None },
            None,
        )
        .await
        .expect("owner may list projects");
    assert!(
        listed.projects.iter().any(|p| p.project_id == project_id),
        "the created project lists back from the production substrate"
    );

    // (3) TENANT SCOPING. A caller in a different tenant must not observe the
    // owner's project — the repository partitions by the per-call tenant.
    let other_tenant = ProductSurfaceCaller::new(
        TenantId::new("prod-projects-other-tenant").unwrap(),
        UserId::new("prod-projects-other-user").unwrap(),
        Some(AgentId::new(RUNTIME_AGENT).unwrap()),
        None,
    );
    let other_surface = ironclaw_host_api::BoundProductSurface::new(bundle.clone(), other_tenant);
    let other_listed = PROJECTS_VIEW
        .query_on(
            &other_surface,
            RebornListProjectsRequest { limit: None },
            None,
        )
        .await
        .expect("a foreign-tenant list is still reachable");
    assert!(
        !other_listed
            .projects
            .iter()
            .any(|p| p.project_id == project_id),
        "a different tenant must not see the owner's project (per-tenant scoping)"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}
