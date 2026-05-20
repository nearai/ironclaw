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
    runtime_policy::{FilesystemBackendKind, NetworkMode, ProcessBackendKind},
};
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
}

impl fmt::Debug for InvocationServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationServices")
            .field("filesystem", &"<root filesystem>")
            .field(
                "runtime_http_egress",
                &self
                    .runtime_http_egress
                    .as_ref()
                    .map(|_| "<runtime http egress>"),
            )
            .field("process", &"<runtime process port>")
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
pub enum InvocationServicesError {
    #[error("filesystem backend {backend:?} is not supported by this invocation services resolver")]
    UnsupportedFilesystemBackend { backend: FilesystemBackendKind },
    #[error("process backend {backend:?} is not supported by this invocation services resolver")]
    UnsupportedProcessBackend { backend: ProcessBackendKind },
    #[error("network mode {mode:?} is not supported by this invocation services resolver")]
    UnsupportedNetworkMode { mode: NetworkMode },
}

impl InvocationServicesError {
    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        match self {
            Self::UnsupportedFilesystemBackend { .. } => RuntimeDispatchErrorKind::FilesystemDenied,
            Self::UnsupportedProcessBackend { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
            Self::UnsupportedNetworkMode { .. } => RuntimeDispatchErrorKind::NetworkDenied,
        }
    }
}

/// Local-host implementation for plans whose required backends are local.
#[derive(Clone)]
pub struct LocalInvocationServicesResolver {
    filesystem: Arc<dyn RootFilesystem>,
    runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    process: Arc<dyn RuntimeProcessPort>,
}

impl LocalInvocationServicesResolver {
    pub fn new(
        filesystem: Arc<dyn RootFilesystem>,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
        process: Arc<dyn RuntimeProcessPort>,
    ) -> Self {
        Self {
            filesystem,
            runtime_http_egress,
            process,
        }
    }
}

impl InvocationServicesResolver for LocalInvocationServicesResolver {
    fn resolve(
        &self,
        request: InvocationServicesResolutionRequest<'_>,
    ) -> Result<InvocationServices, InvocationServicesError> {
        let plan = request.plan;
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
        if plan.requires_process && !matches!(plan.process_backend, ProcessBackendKind::LocalHost) {
            return Err(InvocationServicesError::UnsupportedProcessBackend {
                backend: plan.process_backend,
            });
        }
        if plan.requires_network && matches!(plan.network_mode, NetworkMode::Deny) {
            return Err(InvocationServicesError::UnsupportedNetworkMode {
                mode: plan.network_mode,
            });
        }
        Ok(InvocationServices {
            filesystem: Arc::clone(&self.filesystem),
            runtime_http_egress: self.runtime_http_egress.clone(),
            process: Arc::clone(&self.process),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_api::{CapabilityId, ResourceScope, runtime_policy::SecretMode};

    use crate::{
        CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, RuntimeProcessPort,
    };

    #[derive(Debug)]
    struct NoopProcessPort;

    #[async_trait]
    impl RuntimeProcessPort for NoopProcessPort {
        async fn run_command(
            &self,
            _request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            unreachable!("resolver tests must not execute commands")
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

    #[test]
    fn local_resolver_does_not_require_process_for_pure_plan() {
        let resolver = resolver_without_http();
        let plan = plan(ProcessBackendKind::None, false, false, NetworkMode::Deny);

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
        let mut plan = plan(ProcessBackendKind::None, false, false, NetworkMode::Deny);
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
    fn local_resolver_rejects_denied_required_network() {
        let resolver = resolver_without_http();
        let plan = plan(ProcessBackendKind::None, false, true, NetworkMode::Deny);

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
    fn first_party_tools_do_not_select_process_backends() {
        let sources = [
            include_str!("first_party_tools/shell.rs"),
            include_str!("first_party_tools/http.rs"),
            include_str!("first_party_tools/coding/apply_patch.rs"),
            include_str!("first_party_tools/coding/glob.rs"),
            include_str!("first_party_tools/coding/grep.rs"),
            include_str!("first_party_tools/coding/list_dir.rs"),
            include_str!("first_party_tools/coding/read_file.rs"),
            include_str!("first_party_tools/coding/write_file.rs"),
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
        )
    }

    fn plan(
        process_backend: ProcessBackendKind,
        requires_process: bool,
        requires_network: bool,
        network_mode: NetworkMode,
    ) -> ExecutionPlan {
        ExecutionPlan {
            capability: CapabilityId::new("test.capability".to_string()).unwrap(),
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend,
            network_mode,
            secret_mode: SecretMode::ScrubbedEnv,
            requires_filesystem: false,
            requires_process,
            requires_network,
            requires_secret: false,
        }
    }
}
