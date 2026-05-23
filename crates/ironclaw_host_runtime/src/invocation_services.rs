//! Host API service bindings resolved for one invocation.
//!
//! Capability manifests remain the declaration layer for required host APIs.
//! This module contains the concrete binding layer: after policy/planning and
//! run-profile resolution approve an invocation, composition supplies these
//! services to runtime adapters. First-party handlers consume the Rust traits
//! directly; Script, WASM, MCP, and command-backed adapters should adapt the same
//! bindings into their runtime-specific host APIs rather than resolve placement
//! independently.

use std::{fmt, sync::Arc};

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    MountView, ResourceScope, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    runtime_policy::{
        DeploymentMode, FilesystemBackendKind, NetworkMode, ProcessBackendKind, SecretMode,
    },
};
use ironclaw_secrets::SecretStore;
use thiserror::Error;

use crate::{ExecutionPlan, RuntimeProcessPort};

/// Concrete host API bindings for an already-authorized invocation.
///
/// This type is intentionally runtime-agnostic. It represents the approved
/// host API services for a run profile, not a new capability taxonomy.
#[derive(Clone)]
#[non_exhaustive]
pub struct InvocationServices {
    pub filesystem: Arc<dyn RootFilesystem>,
    pub runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    pub process: Arc<dyn RuntimeProcessPort>,
    pub secret_store: Option<Arc<dyn SecretStore>>,
}

impl fmt::Debug for InvocationServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationServices")
            .field("filesystem", &"[REDACTED]")
            .field(
                "runtime_http_egress",
                &self.runtime_http_egress.as_ref().map(|_| "[REDACTED]"),
            )
            .field("process", &"[REDACTED]")
            .field(
                "secret_store",
                &self.secret_store.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Inputs used to bind an approved execution plan to concrete host services.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct InvocationServicesResolutionRequest<'a> {
    pub plan: &'a ExecutionPlan,
    pub scope: &'a ResourceScope,
    pub mounts: Option<&'a MountView>,
}

/// Resolves concrete host API services for one planned invocation.
///
/// Resolver implementations are the only layer that should inspect backend
/// kinds. Tool handlers and runtime adapters consume the returned services and
/// must not decide local-vs-sandbox placement themselves.
pub trait InvocationServicesResolver: Send + Sync {
    fn resolve(
        &self,
        request: InvocationServicesResolutionRequest<'_>,
    ) -> Result<InvocationServices, InvocationServicesError>;
}

/// Stable redacted service-resolution failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum InvocationServicesError {
    #[error("filesystem backend {backend:?} is not supported by this invocation services resolver")]
    UnsupportedFilesystemBackend { backend: FilesystemBackendKind },
    #[error("process backend {backend:?} is not supported by this invocation services resolver")]
    UnsupportedProcessBackend { backend: ProcessBackendKind },
    #[error("network mode {mode:?} is not supported by this invocation services resolver")]
    UnsupportedNetworkMode { mode: NetworkMode },
    #[error("secret mode {mode:?} is not supported by this invocation services resolver")]
    UnsupportedSecretMode { mode: SecretMode },
    #[error("capability requires secret access but no secret store is configured")]
    SecretAccessRequired,
}

impl InvocationServicesError {
    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        match self {
            Self::UnsupportedFilesystemBackend { .. } => RuntimeDispatchErrorKind::FilesystemDenied,
            Self::UnsupportedProcessBackend { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
            Self::UnsupportedNetworkMode { .. } => RuntimeDispatchErrorKind::NetworkDenied,
            Self::UnsupportedSecretMode { .. } => RuntimeDispatchErrorKind::SecretDenied,
            Self::SecretAccessRequired => RuntimeDispatchErrorKind::SecretDenied,
        }
    }
}

/// Local-host implementation for plans whose required backends are local.
#[derive(Clone)]
pub struct LocalInvocationServicesResolver {
    filesystem: Arc<dyn RootFilesystem>,
    runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    process: Arc<dyn RuntimeProcessPort>,
    tenant_sandbox_process: Option<Arc<dyn RuntimeProcessPort>>,
    secret_store: Option<Arc<dyn SecretStore>>,
}

impl LocalInvocationServicesResolver {
    pub fn new(
        filesystem: Arc<dyn RootFilesystem>,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
        process: Arc<dyn RuntimeProcessPort>,
        secret_store: Option<Arc<dyn SecretStore>>,
    ) -> Self {
        Self {
            filesystem,
            runtime_http_egress,
            process,
            tenant_sandbox_process: None,
            secret_store,
        }
    }

    pub fn with_tenant_sandbox_process_port(
        mut self,
        process: Arc<dyn RuntimeProcessPort>,
    ) -> Self {
        self.tenant_sandbox_process = Some(process);
        self
    }
}

impl InvocationServicesResolver for LocalInvocationServicesResolver {
    fn resolve(
        &self,
        request: InvocationServicesResolutionRequest<'_>,
    ) -> Result<InvocationServices, InvocationServicesError> {
        let plan = request.plan;
        if !matches!(plan.deployment, DeploymentMode::LocalSingleUser) {
            return Err(unsupported_non_local_plan(plan));
        }
        if plan.requires_filesystem
            && !matches!(
                plan.filesystem_backend,
                FilesystemBackendKind::HostWorkspace
            )
        {
            return Err(InvocationServicesError::UnsupportedFilesystemBackend {
                backend: plan.filesystem_backend,
            });
        }
        let process = if plan.requires_process {
            match plan.process_backend {
                ProcessBackendKind::LocalHost => Arc::clone(&self.process),
                ProcessBackendKind::TenantSandbox => self.tenant_sandbox_process.clone().ok_or(
                    InvocationServicesError::UnsupportedProcessBackend {
                        backend: plan.process_backend,
                    },
                )?,
                _ => {
                    return Err(InvocationServicesError::UnsupportedProcessBackend {
                        backend: plan.process_backend,
                    });
                }
            }
        } else {
            Arc::clone(&self.process)
        };
        if plan.requires_network
            && !matches!(
                plan.network_mode,
                NetworkMode::Brokered | NetworkMode::DirectLogged | NetworkMode::Direct
            )
        {
            return Err(InvocationServicesError::UnsupportedNetworkMode {
                mode: plan.network_mode,
            });
        }
        if plan.requires_network && self.runtime_http_egress.is_none() {
            return Err(InvocationServicesError::UnsupportedNetworkMode {
                mode: plan.network_mode,
            });
        }
        if plan.requires_secret
            && !matches!(
                plan.secret_mode,
                SecretMode::ScrubbedEnv | SecretMode::InheritedEnv
            )
        {
            return Err(InvocationServicesError::UnsupportedSecretMode {
                mode: plan.secret_mode,
            });
        }
        if plan.requires_secret && self.secret_store.is_none() {
            return Err(InvocationServicesError::SecretAccessRequired);
        }
        Ok(InvocationServices {
            filesystem: Arc::clone(&self.filesystem),
            runtime_http_egress: plan
                .requires_network
                .then(|| self.runtime_http_egress.clone())
                .flatten(),
            process,
            secret_store: if plan.requires_secret {
                self.secret_store.clone()
            } else {
                None
            },
        })
    }
}

fn unsupported_non_local_plan(plan: &ExecutionPlan) -> InvocationServicesError {
    if plan.requires_filesystem {
        return InvocationServicesError::UnsupportedFilesystemBackend {
            backend: plan.filesystem_backend,
        };
    }
    if plan.requires_process {
        return InvocationServicesError::UnsupportedProcessBackend {
            backend: plan.process_backend,
        };
    }
    if plan.requires_network {
        return InvocationServicesError::UnsupportedNetworkMode {
            mode: plan.network_mode,
        };
    }
    if plan.requires_secret {
        return InvocationServicesError::UnsupportedSecretMode {
            mode: plan.secret_mode,
        };
    }
    InvocationServicesError::UnsupportedProcessBackend {
        backend: plan.process_backend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_api::{
        CapabilityId, ResourceScope,
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

    impl ironclaw_host_api::RuntimeHttpEgress for NoopRuntimeHttpEgress {
        fn execute(
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
            Arc::new(LocalFilesystem::new()),
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
    fn local_resolver_rejects_hosted_brokered_required_network() {
        let resolver = LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
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
                mode: NetworkMode::Brokered
            }
        ));
    }

    #[test]
    fn local_resolver_accepts_direct_required_network_with_egress_service() {
        let resolver = LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
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
    fn local_resolver_hides_runtime_http_egress_when_network_is_not_required() {
        let resolver = LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
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
            Arc::new(LocalFilesystem::new()),
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
    fn local_resolver_rejects_brokered_required_secret_even_with_secret_store() {
        let resolver = LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
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
                mode: SecretMode::BrokeredHandles
            }
        ));
    }

    #[test]
    fn local_resolver_accepts_required_secret_when_secret_store_is_available() {
        let resolver = LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
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
            include_str!("first_party_tools/shell.rs"),
            include_str!("first_party_tools/http.rs"),
        ];
        for source in sources {
            assert!(!source.contains("ProcessBackendKind"));
            assert!(!source.contains("FilesystemBackendKind"));
        }
    }

    fn resolver_without_http() -> LocalInvocationServicesResolver {
        LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(NoopProcessPort),
            None,
        )
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
}
