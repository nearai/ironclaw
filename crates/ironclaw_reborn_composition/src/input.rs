use std::path::PathBuf;
use std::sync::Arc;

use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
use ironclaw_host_runtime::{
    SchedulerTurnRunWakeNotifier, TenantSandboxProcessPort, VerifiedTenantSandboxProcessPort,
};
use ironclaw_trust::HostTrustPolicy;

use crate::{RebornCompositionProfile, RebornProductAuthServices};

#[derive(Clone, Default)]
pub struct RebornHostRuntimePorts {
    pub(crate) tenant_sandbox_process_port: Option<Arc<TenantSandboxProcessPort>>,
    pub(crate) verified_tenant_sandbox_process_port: Option<VerifiedTenantSandboxProcessPort>,
}

impl RebornHostRuntimePorts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tenant_sandbox_process_port(
        mut self,
        process_port: Arc<TenantSandboxProcessPort>,
    ) -> Self {
        self.tenant_sandbox_process_port = Some(process_port);
        self.verified_tenant_sandbox_process_port = None;
        self
    }

    pub fn with_verified_tenant_sandbox_process_port(
        mut self,
        process_port: VerifiedTenantSandboxProcessPort,
    ) -> Self {
        self.verified_tenant_sandbox_process_port = Some(process_port);
        self.tenant_sandbox_process_port = None;
        self
    }
}

pub struct RebornBuildInput {
    pub(crate) profile: RebornCompositionProfile,
    pub(crate) owner_id: String,
    pub(crate) storage: RebornStorageInput,
    pub(crate) production_trust_policy: Option<Arc<HostTrustPolicy>>,
    pub(crate) runtime_policy: Option<EffectiveRuntimePolicy>,
    pub(crate) turn_run_wake_notifier: Option<Arc<SchedulerTurnRunWakeNotifier>>,
    pub(crate) host_runtime_ports: RebornHostRuntimePorts,
    pub(crate) required_runtime_backends: Vec<ironclaw_host_api::RuntimeKind>,
    pub(crate) require_runtime_http_egress: bool,
    pub(crate) require_wasm_credentials: bool,
    pub(crate) product_auth_services: Option<Arc<RebornProductAuthServices>>,
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
        Self::new(
            RebornCompositionProfile::LocalDev,
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

    pub fn with_host_runtime_ports(mut self, ports: RebornHostRuntimePorts) -> Self {
        self.host_runtime_ports = ports;
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

    /// Inject a Reborn-native product-auth composition bundle.
    ///
    /// Production callers should provide durable implementations here once the
    /// auth-flow and credential-account storage substrate is available. The
    /// composition root never falls back to V1 route state or V1 secret stores.
    pub fn with_product_auth_services(mut self, services: Arc<RebornProductAuthServices>) -> Self {
        self.product_auth_services = Some(services);
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
            host_runtime_ports: RebornHostRuntimePorts::default(),
            required_runtime_backends: Vec::new(),
            require_runtime_http_egress: false,
            require_wasm_credentials: false,
            product_auth_services: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_auth::InMemoryAuthProductServices;

    use super::*;

    #[test]
    fn with_product_auth_services_records_injected_bundle() {
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(Arc::new(
            InMemoryAuthProductServices::new(),
        )));

        let input = RebornBuildInput::disabled("test-owner")
            .with_product_auth_services(Arc::clone(&product_auth));

        assert!(Arc::ptr_eq(
            input
                .product_auth_services
                .as_ref()
                .expect("builder should retain injected product auth services"),
            &product_auth
        ));
    }
}
