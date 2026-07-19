use super::*;
use async_trait::async_trait;
use ironclaw_filesystem::{
    DiskFilesystem, FilesystemError, FilesystemOperation, InMemoryBackend, RootFilesystem,
};
use ironclaw_host_api::{
    CapabilityId, MountAlias, MountGrant, MountPermissions, ResourceScope, VirtualPath,
    runtime_policy::{RuntimeProfile, SecretMode},
};
use ironclaw_secrets::InMemorySecretStore;

use crate::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, RuntimeProcessPort,
};

#[derive(Debug)]
struct NoopProcessPort;

#[derive(Debug)]
struct NamedProcessPort(&'static str);

#[async_trait]
impl RuntimeProcessPort for NoopProcessPort {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        unreachable!("resolver tests must not execute commands")
    }
}

struct NoopRuntimeHttpEgress;

#[async_trait]
impl ironclaw_host_api::RuntimeHttpEgress for NoopRuntimeHttpEgress {
    async fn execute(
        &self,
        _request: ironclaw_host_api::RuntimeHttpEgressRequest,
    ) -> Result<
        ironclaw_host_api::RuntimeHttpEgressResponse,
        ironclaw_host_api::RuntimeHttpEgressError,
    > {
        unreachable!("resolver tests must not execute HTTP requests")
    }
}

#[async_trait]
impl RuntimeProcessPort for NamedProcessPort {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: self.0.to_string(),
            saved_output: None,
            exit_code: 0,
            sandboxed: false,
            duration: std::time::Duration::ZERO,
        })
    }
}

#[test]
fn local_resolver_accepts_local_required_process_backend() {
    let resolver = resolver_without_http();
    let plan = plan(
        ProcessBackendKind::LocalHost,
        true,
        false,
        NetworkMode::DirectLogged,
        false,
    );

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();

    assert!(services.runtime_http_egress.is_none());
}

#[test]
fn local_resolver_rejects_hosted_local_host_process_backend() {
    let resolver = resolver_without_http();
    let mut plan = plan(
        ProcessBackendKind::LocalHost,
        true,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::HostedSafe;

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::UnsupportedRunner);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedProcessBackend {
            backend: ProcessBackendKind::LocalHost
        }
    ));
}

#[tokio::test]
async fn local_resolver_routes_post_edit_check_to_the_deployment_isolated_process_port() {
    // The post-edit check rides filesystem-only edit plans, so it must NOT run
    // through the deployment-blind local `process` port. The resolver bundles it
    // with the port matching the plan's process backend: the local host port
    // under LocalSingleUser+LocalHost, the tenant-sandbox port under a hosted
    // tenant-sandbox deployment (so a tenant's command runs isolated in that
    // tenant's sandbox, never on the provider host), and nothing when no backend
    // can run it in isolation.
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        None,
        Arc::new(NamedProcessPort("local-host")),
        None,
    )
    .with_post_edit_check(PostEditCheckConfig::new(
        "cargo check",
        std::time::Duration::from_secs(30),
    ))
    .with_tenant_sandbox_process_port(Arc::new(NamedProcessPort("tenant-sandbox")));

    let resolve = |process_backend, deployment, resolved_profile| {
        let mut plan = plan(process_backend, false, false, NetworkMode::Deny, false);
        plan.deployment = deployment;
        plan.resolved_profile = resolved_profile;
        resolver
            .resolve(InvocationServicesResolutionRequest {
                plan: &plan,
                scope: &ResourceScope::system(),
                mounts: None,
            })
            .expect("non-process plans must still resolve")
    };

    // Run the bundled check port and return the port's identifying output, so we
    // can prove WHICH backend the check would spawn on.
    async fn bundled_port_name(services: &InvocationServices) -> Option<String> {
        let service = services.post_edit_check.as_ref()?;
        let output = service
            .process
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "cargo check".to_string(),
                workdir: None,
                timeout_secs: Some(30),
                extra_env: std::collections::HashMap::new(),
            })
            .await
            .expect("named test port runs");
        Some(output.output)
    }

    let local = resolve(
        ProcessBackendKind::LocalHost,
        DeploymentMode::LocalSingleUser,
        RuntimeProfile::LocalDev,
    );
    assert_eq!(
        bundled_port_name(&local).await.as_deref(),
        Some("local-host"),
        "local single-user runs the check on the local host port"
    );

    let tenant_sandbox = resolve(
        ProcessBackendKind::TenantSandbox,
        DeploymentMode::HostedMultiTenant,
        RuntimeProfile::HostedDev,
    );
    assert_eq!(
        bundled_port_name(&tenant_sandbox).await.as_deref(),
        Some("tenant-sandbox"),
        "hosted multi-tenant runs the check ISOLATED in the tenant sandbox, \
         never on the provider host"
    );

    let no_process = resolve(
        ProcessBackendKind::None,
        DeploymentMode::LocalSingleUser,
        RuntimeProfile::SecureDefault,
    );
    assert!(
        no_process.post_edit_check.is_none(),
        "ProcessBackendKind::None must withhold the post-edit check"
    );

    let hosted_local_host = resolve(
        ProcessBackendKind::LocalHost,
        DeploymentMode::HostedMultiTenant,
        RuntimeProfile::HostedDev,
    );
    assert!(
        hosted_local_host.post_edit_check.is_none(),
        "hosted deployments must never run the check on the provider host"
    );
}

#[test]
fn local_resolver_rejects_sandbox_process_backend_without_local_fallback() {
    let resolver = resolver_without_http();
    let plan = plan(
        ProcessBackendKind::TenantSandbox,
        true,
        false,
        NetworkMode::Allowlist,
        false,
    );

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::UnsupportedRunner);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedProcessBackend {
            backend: ProcessBackendKind::TenantSandbox
        }
    ));
}

#[tokio::test]
async fn local_resolver_uses_configured_sandbox_process_backend() {
    let resolver = resolver_without_http()
        .with_tenant_sandbox_process_port(Arc::new(NamedProcessPort("sandbox")));
    let plan = plan(
        ProcessBackendKind::TenantSandbox,
        true,
        false,
        NetworkMode::Allowlist,
        false,
    );

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();

    let output = services
        .process
        .run_command(CommandExecutionRequest {
            scope: ResourceScope::system(),
            mounts: None,
            command: "echo hi".to_string(),
            workdir: None,
            timeout_secs: None,
            extra_env: Default::default(),
        })
        .await
        .unwrap();
    assert_eq!(output.output, "sandbox");
}

#[test]
fn local_resolver_rejects_unsupported_required_process_backend() {
    let resolver = resolver_without_http();
    let plan = plan(
        ProcessBackendKind::Docker,
        true,
        false,
        NetworkMode::Deny,
        false,
    );

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::UnsupportedRunner);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedProcessBackend {
            backend: ProcessBackendKind::Docker
        }
    ));
}

#[test]
fn local_resolver_does_not_require_process_for_pure_plan() {
    let resolver = resolver_without_http();
    let plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );

    resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();
}

#[test]
fn local_resolver_rejects_unsupported_filesystem_backend() {
    let resolver = resolver_without_http();
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.requires_filesystem = true;
    plan.filesystem_backend = FilesystemBackendKind::TenantWorkspace;

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::FilesystemDenied);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedFilesystemBackend {
            backend: FilesystemBackendKind::TenantWorkspace
        }
    ));
}

#[test]
fn local_resolver_accepts_host_workspace_and_home_when_filesystem_required() {
    let resolver = resolver_without_http();
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.requires_filesystem = true;
    plan.filesystem_backend = FilesystemBackendKind::HostWorkspaceAndHome;

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .expect("local-yolo filesystem backend should resolve");

    assert!(services.runtime_http_egress.is_none());
}

#[test]
fn local_resolver_rejects_hosted_raw_host_filesystem_backends() {
    let resolver = resolver_without_http();
    for filesystem_backend in [
        FilesystemBackendKind::HostWorkspace,
        FilesystemBackendKind::HostWorkspaceAndHome,
    ] {
        let mut plan = plan(
            ProcessBackendKind::None,
            false,
            false,
            NetworkMode::Deny,
            false,
        );
        plan.deployment = DeploymentMode::HostedMultiTenant;
        plan.resolved_profile = RuntimeProfile::HostedSafe;
        plan.requires_filesystem = true;
        plan.filesystem_backend = filesystem_backend;

        let error = resolver
            .resolve(InvocationServicesResolutionRequest {
                plan: &plan,
                scope: &ResourceScope::system(),
                mounts: None,
            })
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::FilesystemDenied);
        assert!(matches!(
            error,
            InvocationServicesError::UnsupportedFilesystemBackend { backend }
                if backend == filesystem_backend
        ));
    }
}

#[test]
fn local_resolver_accepts_hosted_scoped_virtual_filesystem_with_mounts() {
    let resolver = resolver_without_http();
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::SecureDefault;
    plan.requires_filesystem = true;
    plan.filesystem_backend = FilesystemBackendKind::ScopedVirtual;
    let mounts = scoped_mount_view();

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: Some(&mounts),
        })
        .expect("hosted scoped virtual filesystem should resolve with explicit mounts");

    assert!(services.runtime_http_egress.is_none());
}

#[tokio::test]
async fn hosted_scoped_virtual_filesystem_services_enforce_mount_targets() {
    let backing: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    backing
        .write_file(&vpath("/system/extensions/catalog.json"), b"{\"ok\":true}")
        .await
        .unwrap();
    backing
        .write_file(&vpath("/users/user_test/private.txt"), b"secret")
        .await
        .unwrap();
    let resolver = resolver_with_filesystem(Arc::clone(&backing));
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::SecureDefault;
    plan.requires_filesystem = true;
    plan.filesystem_backend = FilesystemBackendKind::ScopedVirtual;
    let mounts = scoped_mount_view();

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: Some(&mounts),
        })
        .expect("hosted scoped virtual filesystem should resolve with explicit mounts");

    let allowed = services
        .filesystem
        .read_file(&vpath("/system/extensions/catalog.json"))
        .await
        .unwrap();
    assert_eq!(allowed, b"{\"ok\":true}");

    let denied = services
        .filesystem
        .read_file(&vpath("/users/user_test/private.txt"))
        .await
        .unwrap_err();
    assert_permission_denied(denied, FilesystemOperation::ReadFile);
}

#[tokio::test]
async fn hosted_scoped_virtual_filesystem_services_enforce_mount_permissions() {
    let resolver = resolver_with_filesystem(Arc::new(InMemoryBackend::new()));
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::SecureDefault;
    plan.requires_filesystem = true;
    plan.filesystem_backend = FilesystemBackendKind::ScopedVirtual;
    let mounts = scoped_mount_view();

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: Some(&mounts),
        })
        .expect("hosted scoped virtual filesystem should resolve with explicit mounts");

    let denied = services
        .filesystem
        .write_file(&vpath("/system/extensions/state.json"), b"mutate")
        .await
        .unwrap_err();
    assert_permission_denied(denied, FilesystemOperation::WriteFile);
}

#[tokio::test]
async fn hosted_scoped_virtual_filesystem_delete_if_version_delegates_to_inner_backend() {
    // Review fix (PR #5749, round 3): MountScopedRootFilesystem must forward
    // delete_if_version to the inner backend (after the same permission
    // resolution as `delete`) instead of falling through to the
    // RootFilesystem trait default `Unsupported`.
    use ironclaw_filesystem::{CasExpectation, Entry};

    let resolver = resolver_with_filesystem(Arc::new(InMemoryBackend::new()));
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::SecureDefault;
    plan.requires_filesystem = true;
    plan.filesystem_backend = FilesystemBackendKind::ScopedVirtual;
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/system/extensions".to_string()).expect("mount alias"),
        VirtualPath::new("/system/extensions".to_string()).expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: Some(&mounts),
        })
        .expect("hosted scoped virtual filesystem should resolve with explicit mounts");

    let path = vpath("/system/extensions/catalog.json");
    let version = services
        .filesystem
        .put(
            &path,
            Entry::bytes(b"{\"ok\":true}".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    // Wrong version is rejected with VersionMismatch, proving the call
    // actually reached the inner backend's CAS logic rather than a stub or
    // an Unsupported fallthrough.
    let other_version = version.next();
    let err = services
        .filesystem
        .delete_if_version(&path, other_version)
        .await
        .unwrap_err();
    assert!(matches!(err, FilesystemError::VersionMismatch { .. }));

    // Correct version deletes.
    services
        .filesystem
        .delete_if_version(&path, version)
        .await
        .unwrap();
    assert!(services.filesystem.read_file(&path).await.is_err());

    // Out-of-mount path is denied before it ever reaches the inner backend.
    let out_of_mount = vpath("/users/user_test/private.txt");
    let denied = services
        .filesystem
        .delete_if_version(&out_of_mount, version)
        .await
        .unwrap_err();
    assert_permission_denied(denied, FilesystemOperation::Delete);
}

#[tokio::test]
async fn hosted_scoped_virtual_filesystem_delete_if_version_denies_when_delete_missing() {
    // Round-C review (PR #5749): the sibling `scoped/tests.rs` suite in
    // ironclaw_filesystem already pins this at the ScopedFilesystem layer
    // (`delete_if_version_denies_when_delete_missing`), but MountScopedRootFilesystem
    // resolves permissions independently — mirror it here so a mount that
    // grants read/write/list but not delete is rejected before any backend
    // dispatch at this layer too, not just proven-by-shared-implementation.
    use ironclaw_filesystem::{CasExpectation, Entry};

    let resolver = resolver_with_filesystem(Arc::new(InMemoryBackend::new()));
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::SecureDefault;
    plan.requires_filesystem = true;
    plan.filesystem_backend = FilesystemBackendKind::ScopedVirtual;
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/system/extensions".to_string()).expect("mount alias"),
        VirtualPath::new("/system/extensions".to_string()).expect("virtual path"),
        MountPermissions::read_write(),
    )])
    .expect("mount view");

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: Some(&mounts),
        })
        .expect("hosted scoped virtual filesystem should resolve with explicit mounts");

    let path = vpath("/system/extensions/catalog.json");
    let version = services
        .filesystem
        .put(
            &path,
            Entry::bytes(b"{\"ok\":true}".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    let denied = services
        .filesystem
        .delete_if_version(&path, version)
        .await
        .unwrap_err();
    assert_permission_denied(denied, FilesystemOperation::Delete);

    // The entry survives the denied delete.
    assert!(services.filesystem.read_file(&path).await.is_ok());
}

#[test]
fn local_resolver_rejects_hosted_scoped_virtual_filesystem_without_mounts() {
    let resolver = resolver_without_http();
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::SecureDefault;
    plan.requires_filesystem = true;
    plan.filesystem_backend = FilesystemBackendKind::ScopedVirtual;

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::FilesystemDenied);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedFilesystemBackend {
            backend: FilesystemBackendKind::ScopedVirtual
        }
    ));
}

#[test]
fn local_resolver_ignores_unsupported_filesystem_backend_when_not_required() {
    let resolver = resolver_without_http();
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.filesystem_backend = FilesystemBackendKind::TenantWorkspace;

    resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .expect("unused filesystem backend must not block pure invocations");
}

#[tokio::test]
async fn unused_filesystem_backend_resolves_to_denied_filesystem_services() {
    let backing: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    backing
        .write_file(&vpath("/system/extensions/catalog.json"), b"catalog")
        .await
        .unwrap();
    let resolver = resolver_with_filesystem(backing);
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );
    plan.filesystem_backend = FilesystemBackendKind::TenantWorkspace;

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .expect("unused filesystem backend must not block pure invocations");

    let denied = services
        .filesystem
        .read_file(&vpath("/system/extensions/catalog.json"))
        .await
        .unwrap_err();
    assert_permission_denied(denied, FilesystemOperation::ReadFile);
}

#[test]
fn local_resolver_rejects_denied_required_network() {
    let resolver = resolver_without_http();
    let plan = plan(
        ProcessBackendKind::None,
        false,
        true,
        NetworkMode::Deny,
        false,
    );

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedNetworkMode {
            mode: NetworkMode::Deny
        }
    ));
}

#[test]
fn local_resolver_rejects_required_network_when_egress_service_is_absent() {
    let resolver = resolver_without_http();
    let plan = plan(
        ProcessBackendKind::None,
        false,
        true,
        NetworkMode::DirectLogged,
        false,
    );

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedNetworkMode {
            mode: NetworkMode::DirectLogged
        }
    ));
}

#[test]
fn local_resolver_accepts_brokered_required_network_with_egress_service() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        Some(Arc::new(NoopRuntimeHttpEgress)),
        Arc::new(NoopProcessPort),
        None,
    );
    let plan = plan(
        ProcessBackendKind::None,
        false,
        true,
        NetworkMode::Brokered,
        false,
    );

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();

    assert!(services.runtime_http_egress.is_some());
}

#[test]
fn local_resolver_accepts_hosted_brokered_required_network() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        Some(Arc::new(NoopRuntimeHttpEgress)),
        Arc::new(NoopProcessPort),
        None,
    );
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        true,
        NetworkMode::Brokered,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::HostedSafe;

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .expect("hosted brokered network should resolve through host egress");

    assert!(services.runtime_http_egress.is_some());
}

#[test]
fn local_resolver_accepts_hosted_and_enterprise_allowlist_required_network() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        Some(Arc::new(NoopRuntimeHttpEgress)),
        Arc::new(NoopProcessPort),
        None,
    );
    for (deployment, profile) in [
        (DeploymentMode::HostedMultiTenant, RuntimeProfile::HostedDev),
        (
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseDev,
        ),
    ] {
        let mut plan = plan(
            ProcessBackendKind::None,
            false,
            true,
            NetworkMode::Allowlist,
            false,
        );
        plan.deployment = deployment;
        plan.resolved_profile = profile;

        let services = resolver
            .resolve(InvocationServicesResolutionRequest {
                plan: &plan,
                scope: &ResourceScope::system(),
                mounts: None,
            })
            .expect("allowlisted network should resolve through host egress");

        assert!(services.runtime_http_egress.is_some());
    }
}

#[test]
fn local_resolver_rejects_hosted_direct_required_network() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        Some(Arc::new(NoopRuntimeHttpEgress)),
        Arc::new(NoopProcessPort),
        None,
    );
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        true,
        NetworkMode::Direct,
        false,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::HostedSafe;

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedNetworkMode {
            mode: NetworkMode::Direct
        }
    ));
}

#[test]
fn local_resolver_accepts_direct_required_network_with_egress_service() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        Some(Arc::new(NoopRuntimeHttpEgress)),
        Arc::new(NoopProcessPort),
        None,
    );
    let plan = plan(
        ProcessBackendKind::None,
        false,
        true,
        NetworkMode::Direct,
        false,
    );

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();

    assert!(services.runtime_http_egress.is_some());
}

#[test]
fn local_resolver_allows_raw_diagnostics_only_for_local_dev_and_yolo() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        Some(Arc::new(NoopRuntimeHttpEgress)),
        Arc::new(NoopProcessPort),
        None,
    );
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        true,
        NetworkMode::Direct,
        false,
    );

    plan.resolved_profile = RuntimeProfile::LocalSafe;
    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();
    assert!(!services.unsafe_raw_diagnostics_allowed);

    plan.resolved_profile = RuntimeProfile::LocalDev;
    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();
    assert!(services.unsafe_raw_diagnostics_allowed);

    plan.resolved_profile = RuntimeProfile::LocalYolo;
    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();
    assert!(services.unsafe_raw_diagnostics_allowed);
}

#[test]
fn local_resolver_hides_runtime_http_egress_when_network_is_not_required() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        Some(Arc::new(NoopRuntimeHttpEgress)),
        Arc::new(NoopProcessPort),
        None,
    );
    let plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::DirectLogged,
        false,
    );

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();

    assert!(services.runtime_http_egress.is_none());
}

#[test]
fn local_resolver_hides_secret_store_when_secret_is_not_required() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        None,
        Arc::new(NoopProcessPort),
        Some(Arc::new(InMemorySecretStore::new())),
    );
    let plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        false,
    );

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();

    assert!(services.secret_store.is_none());
}

#[test]
fn local_resolver_rejects_required_secret_when_secret_store_is_absent() {
    let resolver = resolver_without_http();
    let plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        true,
    );

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::SecretDenied);
    assert!(matches!(
        error,
        InvocationServicesError::SecretAccessRequired
    ));
}

#[test]
fn local_resolver_accepts_brokered_required_secret_with_secret_store() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        None,
        Arc::new(NoopProcessPort),
        Some(Arc::new(InMemorySecretStore::new())),
    );
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        true,
    );
    plan.secret_mode = SecretMode::BrokeredHandles;

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .expect("brokered handles should resolve with a configured secret store");

    assert!(services.secret_store.is_some());
}

#[test]
fn local_resolver_accepts_tenant_and_org_broker_required_secrets() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        None,
        Arc::new(NoopProcessPort),
        Some(Arc::new(InMemorySecretStore::new())),
    );
    for (deployment, profile, secret_mode) in [
        (
            DeploymentMode::HostedMultiTenant,
            RuntimeProfile::HostedSafe,
            SecretMode::TenantBroker,
        ),
        (
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseSafe,
            SecretMode::OrgBroker,
        ),
    ] {
        let mut plan = plan(
            ProcessBackendKind::None,
            false,
            false,
            NetworkMode::Deny,
            true,
        );
        plan.deployment = deployment;
        plan.resolved_profile = profile;
        plan.secret_mode = secret_mode;

        let services = resolver
            .resolve(InvocationServicesResolutionRequest {
                plan: &plan,
                scope: &ResourceScope::system(),
                mounts: None,
            })
            .expect("brokered hosted secrets should resolve with a configured secret store");

        assert!(services.secret_store.is_some());
    }
}

#[test]
fn local_resolver_rejects_hosted_inherited_env_secret() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        None,
        Arc::new(NoopProcessPort),
        Some(Arc::new(InMemorySecretStore::new())),
    );
    let mut plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        true,
    );
    plan.deployment = DeploymentMode::HostedMultiTenant;
    plan.resolved_profile = RuntimeProfile::HostedSafe;
    plan.secret_mode = SecretMode::InheritedEnv;

    let error = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::SecretDenied);
    assert!(matches!(
        error,
        InvocationServicesError::UnsupportedSecretMode {
            mode: SecretMode::InheritedEnv
        }
    ));
}

#[test]
fn local_resolver_accepts_required_secret_when_secret_store_is_available() {
    let resolver = LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        None,
        Arc::new(NoopProcessPort),
        Some(Arc::new(InMemorySecretStore::new())),
    );
    let plan = plan(
        ProcessBackendKind::None,
        false,
        false,
        NetworkMode::Deny,
        true,
    );

    let services = resolver
        .resolve(InvocationServicesResolutionRequest {
            plan: &plan,
            scope: &ResourceScope::system(),
            mounts: None,
        })
        .unwrap();

    assert!(services.secret_store.is_some());
}

#[test]
fn first_party_tools_do_not_select_process_backends() {
    let sources = [
        include_str!("../first_party_tools/shell.rs"),
        include_str!("../first_party_tools/http.rs"),
    ];
    for source in sources {
        assert!(!source.contains("ProcessBackendKind"));
        assert!(!source.contains("FilesystemBackendKind"));
        assert!(!source.contains("NetworkMode"));
        assert!(!source.contains("SecretMode"));
    }
}

fn resolver_without_http() -> LocalInvocationServicesResolver {
    LocalInvocationServicesResolver::new(
        Arc::new(DiskFilesystem::new()),
        None,
        Arc::new(NoopProcessPort),
        None,
    )
}

fn resolver_with_filesystem(
    filesystem: Arc<dyn RootFilesystem>,
) -> LocalInvocationServicesResolver {
    LocalInvocationServicesResolver::new(filesystem, None, Arc::new(NoopProcessPort), None)
}

fn scoped_mount_view() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/system/extensions".to_string()).expect("mount alias"),
        VirtualPath::new("/system/extensions".to_string()).expect("virtual path"),
        MountPermissions::read_only(),
    )])
    .expect("mount view")
}

fn vpath(path: &str) -> VirtualPath {
    VirtualPath::new(path.to_string()).expect("virtual path")
}

fn assert_permission_denied(error: FilesystemError, expected_operation: FilesystemOperation) {
    assert!(matches!(
        error,
        FilesystemError::PermissionDenied {
            operation,
            ..
        } if operation == expected_operation
    ));
}

fn plan(
    process_backend: ProcessBackendKind,
    requires_process: bool,
    requires_network: bool,
    network_mode: NetworkMode,
    requires_secret: bool,
) -> ExecutionPlan {
    ExecutionPlan {
        capability: CapabilityId::new("test.capability".to_string()).unwrap(),
        deployment: DeploymentMode::LocalSingleUser,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend,
        network_mode,
        secret_mode: SecretMode::ScrubbedEnv,
        requires_filesystem: false,
        requires_process,
        requires_network,
        requires_secret,
    }
}
