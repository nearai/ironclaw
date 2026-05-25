use std::path::PathBuf;
use std::sync::Arc;

use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::runtime_policy::ProcessBackendKind;
use ironclaw_host_runtime::{SchedulerTurnRunWakeNotifier, TenantSandboxProcessPort};
use ironclaw_trust::HostTrustPolicy;

use crate::{RebornCompositionProfile, RebornProductAuthServicePorts};

#[derive(Clone, Debug, Default)]
pub enum RebornRuntimeProcessBinding {
    #[default]
    None,
    TenantSandbox {
        process_port: Arc<TenantSandboxProcessPort>,
    },
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RebornRuntimeProcessBindingError {
    MissingTenantSandboxProcessPort,
    UnexpectedTenantSandboxProcessPort { process_backend: ProcessBackendKind },
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl std::fmt::Display for RebornRuntimeProcessBindingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingTenantSandboxProcessPort => formatter.write_str(
                "production tenant-sandbox process backend requires a tenant sandbox process binding",
            ),
            Self::UnexpectedTenantSandboxProcessPort { process_backend } => write!(
                formatter,
                "production runtime policy uses {process_backend:?} but a tenant sandbox process binding was supplied"
            ),
        }
    }
}

impl RebornRuntimeProcessBinding {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn tenant_sandbox(process_port: Arc<TenantSandboxProcessPort>) -> Self {
        Self::TenantSandbox { process_port }
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) fn validate_for_production_policy(
        &self,
        runtime_policy: &EffectiveRuntimePolicy,
    ) -> Result<(), RebornRuntimeProcessBindingError> {
        match (runtime_policy.process_backend, self) {
            (
                ProcessBackendKind::TenantSandbox,
                RebornRuntimeProcessBinding::TenantSandbox { .. },
            ) => Ok(()),
            (ProcessBackendKind::TenantSandbox, RebornRuntimeProcessBinding::None) => {
                Err(RebornRuntimeProcessBindingError::MissingTenantSandboxProcessPort)
            }
            (_, RebornRuntimeProcessBinding::TenantSandbox { .. }) => Err(
                RebornRuntimeProcessBindingError::UnexpectedTenantSandboxProcessPort {
                    process_backend: runtime_policy.process_backend,
                },
            ),
            (_, RebornRuntimeProcessBinding::None) => Ok(()),
        }
    }
}

pub struct RebornBuildInput {
    pub(crate) profile: RebornCompositionProfile,
    pub(crate) owner_id: String,
    pub(crate) storage: RebornStorageInput,
    pub(crate) production_trust_policy: Option<Arc<HostTrustPolicy>>,
    pub(crate) runtime_policy: Option<EffectiveRuntimePolicy>,
    pub(crate) turn_run_wake_notifier: Option<Arc<SchedulerTurnRunWakeNotifier>>,
    pub(crate) runtime_process_binding: RebornRuntimeProcessBinding,
    pub(crate) required_runtime_backends: Vec<ironclaw_host_api::RuntimeKind>,
    pub(crate) require_runtime_http_egress: bool,
    pub(crate) require_wasm_credentials: bool,
    pub(crate) product_auth_ports: Option<RebornProductAuthServicePorts>,
}

pub(crate) enum RebornStorageInput {
    Disabled,
    LocalDev {
        root: PathBuf,
        workspace_root: Option<PathBuf>,
    },
    #[cfg(feature = "libsql")]
    Libsql {
        db: Arc<libsql::Database>,
        path_or_url: String,
        auth_token: Option<ironclaw_secrets::SecretMaterial>,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    },
    #[cfg(feature = "postgres")]
    Postgres {
        pool: deadpool_postgres::Pool,
        url: ironclaw_secrets::SecretMaterial,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    },
}

impl RebornBuildInput {
    /// Selected composition profile.
    pub fn profile(&self) -> RebornCompositionProfile {
        self.profile
    }

    /// Owner id (string form). Used by the assembled runtime to mint the
    /// `UserId` actor for inbound CLI messages.
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn disabled(owner_id: impl Into<String>) -> Self {
        Self::new(
            RebornCompositionProfile::Disabled,
            owner_id,
            RebornStorageInput::Disabled,
        )
    }

    pub fn local_dev(owner_id: impl Into<String>, root: PathBuf) -> Self {
        Self::local_dev_with_profile(RebornCompositionProfile::LocalDev, owner_id, root)
    }

    pub(crate) fn local_dev_with_profile(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        root: PathBuf,
    ) -> Self {
        debug_assert!(matches!(
            profile,
            RebornCompositionProfile::LocalDev | RebornCompositionProfile::LocalDevYolo
        ));
        Self::new(
            profile,
            owner_id,
            RebornStorageInput::LocalDev {
                root,
                workspace_root: None,
            },
        )
    }

    pub fn with_local_dev_workspace_root(mut self, workspace_root: PathBuf) -> Self {
        if let RebornStorageInput::LocalDev {
            workspace_root: root,
            ..
        } = &mut self.storage
        {
            *root = Some(workspace_root);
        }
        self
    }

    #[cfg(feature = "libsql")]
    pub fn libsql(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        db: Arc<libsql::Database>,
        path_or_url: impl Into<String>,
        auth_token: Option<ironclaw_secrets::SecretMaterial>,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    ) -> Self {
        Self::new(
            profile,
            owner_id,
            RebornStorageInput::Libsql {
                db,
                path_or_url: path_or_url.into(),
                auth_token,
                secret_master_key,
            },
        )
    }

    #[cfg(feature = "postgres")]
    pub fn postgres(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        pool: deadpool_postgres::Pool,
        url: ironclaw_secrets::SecretMaterial,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    ) -> Self {
        Self::new(
            profile,
            owner_id,
            RebornStorageInput::Postgres {
                pool,
                url,
                secret_master_key,
            },
        )
    }

    pub fn with_required_runtime_backends(
        mut self,
        backends: impl IntoIterator<Item = ironclaw_host_api::RuntimeKind>,
    ) -> Self {
        self.required_runtime_backends = backends.into_iter().collect();
        self
    }

    pub fn with_production_trust_policy(mut self, policy: Arc<HostTrustPolicy>) -> Self {
        self.production_trust_policy = Some(policy);
        self
    }

    pub fn with_runtime_policy(mut self, policy: EffectiveRuntimePolicy) -> Self {
        self.runtime_policy = Some(policy);
        self
    }

    pub fn runtime_policy(&self) -> Option<&EffectiveRuntimePolicy> {
        self.runtime_policy.as_ref()
    }

    pub fn with_turn_run_wake_notifier(
        mut self,
        notifier: Arc<SchedulerTurnRunWakeNotifier>,
    ) -> Self {
        self.turn_run_wake_notifier = Some(notifier);
        self
    }

    pub fn with_runtime_process_binding(mut self, binding: RebornRuntimeProcessBinding) -> Self {
        self.runtime_process_binding = binding;
        self
    }

    pub fn require_runtime_http_egress(mut self) -> Self {
        self.require_runtime_http_egress = true;
        self
    }

    pub fn require_wasm_credentials(mut self) -> Self {
        self.require_wasm_credentials = true;
        self
    }

    /// Inject Reborn-native product-auth service ports.
    ///
    /// Production callers should provide durable implementations here. The
    /// composition root attaches the turn-continuation dispatcher after it has
    /// composed the profile's [`ironclaw_turns::TurnCoordinator`], so OAuth
    /// continuations cannot accidentally bypass the active coordinator.
    pub fn with_product_auth_ports(mut self, ports: RebornProductAuthServicePorts) -> Self {
        self.product_auth_ports = Some(ports);
        self
    }

    fn new(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        storage: RebornStorageInput,
    ) -> Self {
        Self {
            profile,
            owner_id: owner_id.into(),
            storage,
            production_trust_policy: None,
            runtime_policy: None,
            turn_run_wake_notifier: None,
            runtime_process_binding: RebornRuntimeProcessBinding::default(),
            required_runtime_backends: Vec::new(),
            require_runtime_http_egress: false,
            require_wasm_credentials: false,
            product_auth_ports: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_auth::InMemoryAuthProductServices;

    use super::*;

    #[test]
    fn with_product_auth_ports_records_injected_ports() {
        let product_auth = RebornProductAuthServicePorts::from_shared(Arc::new(
            InMemoryAuthProductServices::new(),
        ));

        let input =
            RebornBuildInput::disabled("test-owner").with_product_auth_ports(product_auth.clone());

        assert!(input.product_auth_ports.is_some());
    }
}
