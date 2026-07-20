use std::path::PathBuf;
#[cfg(feature = "postgres")]
use std::str::FromStr;
use std::sync::Arc;

use ironclaw_auth::{AuthProductError, CredentialAccountLabel, OAuthClientId, OAuthRedirectUri};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::runtime_policy::ProcessBackendKind;
#[cfg(feature = "postgres")]
use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};
use ironclaw_host_api::runtime_policy::{
    EffectiveRuntimePolicy, FilesystemBackendKind, NetworkMode, SecretMode,
};
use ironclaw_host_api::{AgentId, TenantId};
#[cfg(test)]
use ironclaw_host_runtime::HostRuntimeHttpEgressPort;
use ironclaw_host_runtime::TenantSandboxProcessPort;
#[cfg(any(test, feature = "test-support"))]
use ironclaw_network::NetworkHttpEgress;
use ironclaw_trust::HostTrustPolicy;
use ironclaw_turns::{InMemoryTurnStateStoreLimits, TurnRunWakeNotifier};
use secrecy::SecretString;

#[cfg(feature = "postgres")]
use ironclaw_reborn_config::StorageBackend;
#[cfg(feature = "postgres")]
use ironclaw_reborn_event_store::{PostgresPoolTlsOptions, RebornPostgresSslMode};

#[cfg(feature = "postgres")]
use crate::RebornBuildError;
use crate::deployment::DeploymentConfig;
use crate::product_auth::oauth::google_oauth::google_provider_spec;
use crate::product_auth::oauth::notion_oauth::notion_provider_spec;
use crate::product_auth::oauth::oauth_dcr::OAuthDcrProviderConfig;
use crate::product_auth::oauth::oauth_provider_client::HostOAuthProviderSpec;
use crate::slack::slack_setup::SlackPersonalSetupServiceSlot;
use crate::{RebornCompositionProfile, RebornProductAuthServicePorts};

#[cfg(feature = "postgres")]
const DEFAULT_REBORN_POSTGRES_URL_ENV: &str = "IRONCLAW_REBORN_POSTGRES_URL";
#[cfg(feature = "postgres")]
const DEFAULT_REBORN_SECRET_MASTER_KEY_ENV: &str = "IRONCLAW_REBORN_SECRET_MASTER_KEY";
#[cfg(feature = "postgres")]
const REBORN_POSTGRES_POOL_MAX_SIZE_ENV: &str = "IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE";
#[cfg(feature = "postgres")]
const REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV: &str =
    "IRONCLAW_REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON";
#[cfg(feature = "postgres")]
const DATABASE_SSLMODE_ENV: &str = "DATABASE_SSLMODE";
#[cfg(feature = "postgres")]
const ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV: &str =
    "IRONCLAW_REBORN_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT";

/// Composition-time OAuth client metadata.
///
/// `RebornBuildInput` owns this seam for product/bootstrap-provided values
/// until a settings-backed source exists.
#[derive(Clone)]
pub struct OAuthClientConfig {
    pub client_id: OAuthClientId,
    pub client_secret: Option<SecretString>,
    pub redirect_uri: OAuthRedirectUri,
    pub hosted_domain_hint: Option<String>,
}

impl OAuthClientConfig {
    pub fn new(
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
        client_secret: Option<SecretString>,
    ) -> Result<Self, AuthProductError> {
        Ok(Self {
            client_id: OAuthClientId::new(client_id)?,
            client_secret,
            redirect_uri: OAuthRedirectUri::new(redirect_uri)?,
            hosted_domain_hint: None,
        })
    }

    pub fn with_hosted_domain_hint(mut self, hosted_domain_hint: impl Into<String>) -> Self {
        self.hosted_domain_hint = Some(hosted_domain_hint.into());
        self
    }
}

impl std::fmt::Debug for OAuthClientConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthClientConfig")
            .field("client_id", &self.client_id.as_str())
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("redirect_uri", &self.redirect_uri)
            .field(
                "hosted_domain_hint",
                &self.hosted_domain_hint.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OAuthProviderBackendConfig {
    pub(crate) spec: HostOAuthProviderSpec,
    pub(crate) client: OAuthClientConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct OAuthDcrProviderBackendConfig {
    pub(crate) config: OAuthDcrProviderConfig,
}

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
    /// The deployment this build assembles, as data (§4.4/§5.6). Carries the
    /// substrate, traffic, readiness, and storage-shape axes every consumer
    /// reads instead of re-deriving them from a profile name.
    ///
    /// The **resolved** runtime policy rides `runtime_policy`, not this value:
    /// `new` builds the config without a yolo host-access disclosure (it is not
    /// known at construction), so callers that hold the operator's confirmation
    /// install the accurate config through
    /// [`RebornBuildInput::with_deployment`] — `local_runtime_build_input_with_options`
    /// is the one that does.
    pub(crate) deployment: DeploymentConfig,
    pub(crate) owner_id: String,
    pub(crate) local_runtime_identity: Option<RebornLocalRuntimeIdentity>,
    pub(crate) storage: RebornStorageInput,
    pub(crate) production_trust_policy: Option<Arc<HostTrustPolicy>>,
    pub(crate) runtime_policy: Option<EffectiveRuntimePolicy>,
    pub(crate) turn_run_wake_notifier: Option<Arc<dyn TurnRunWakeNotifier>>,
    pub(crate) runtime_process_binding: RebornRuntimeProcessBinding,
    pub(crate) required_runtime_backends: Vec<ironclaw_host_api::RuntimeKind>,
    pub(crate) require_runtime_http_egress: bool,
    pub(crate) require_wasm_credentials: bool,
    #[cfg(test)]
    pub(crate) host_runtime_http_egress_for_test: Option<Option<HostRuntimeHttpEgressPort>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) network_http_egress_for_test: Option<Arc<dyn NetworkHttpEgress>>,
    pub(crate) product_auth_ports: Option<RebornProductAuthServicePorts>,
    pub(crate) oauth_provider_configs: Vec<OAuthProviderBackendConfig>,
    pub(crate) oauth_dcr_provider_configs: Vec<OAuthDcrProviderBackendConfig>,
    pub(crate) slack_personal_oauth_lazy_slot: Option<SlackPersonalSetupServiceSlot>,
    /// Build-time Slack host-beta wiring signal: whether the CLI `serve`
    /// path resolved a Slack
    /// host-beta config for this instance BEFORE the composition build ran.
    /// Mirrors how `google_oauth_configured` arrives via
    /// `oauth_provider_configs` — one signal, read by
    /// `provider_instance_readiness_map` to decide whether the
    /// `slack_personal` provider needs a readiness-map entry. Defaults
    /// `false`; unrelated to whether the Slack host-beta mounts are composed
    /// post-build (a separate, later step — see `serve.rs`).
    pub(crate) slack_host_beta_enabled: bool,
    /// Build-time signal that this instance resolved
    /// `IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI`. Slack personal
    /// OAuth needs this IN ADDITION to `slack_host_beta_enabled`: with the
    /// route mounted but no redirect URI, the WebUI Connect button reaches
    /// `product_auth::serve::slack_personal_oauth_credentials` and gets a
    /// message-less 503.
    ///
    /// Deliberately NOT derived from `slack_personal_oauth_lazy_slot`, even
    /// though the CLI resolves both from that one env var: the slot is a
    /// composition input that switches the Slack provider client to
    /// lazy setup-service credential resolution, so deriving readiness from it
    /// would force every fixture that merely wants "this instance is
    /// configured" to also opt into lazy credentials it never fills (proved by
    /// `factory::auth_tests::slack_oauth_callback_activates_and_publishes_all_personal_tools`,
    /// which fails `BackendUnavailable` that way). This field records the
    /// operator FACT; the slot performs the WIRING. Defaults `false`.
    pub(crate) slack_personal_oauth_redirect_uri_configured: bool,
    pub(crate) nearai_mcp_bootstrap_config:
        Option<crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig>,
    /// Concurrency limits applied to the in-memory turn-state store.
    /// Defaults to no limits (all caps `None` / unlimited).
    pub(crate) turn_state_store_limits: InMemoryTurnStateStoreLimits,
}

#[derive(Clone, Debug)]
pub(crate) struct RebornLocalRuntimeIdentity {
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
}

pub(crate) enum RebornStorageInput {
    Disabled,
    LocalDev {
        root: PathBuf,
        workspace_root: Option<PathBuf>,
        host_home_root: Option<PathBuf>,
    },
    #[cfg(feature = "postgres")]
    HostedSingleTenantPostgres {
        root: PathBuf,
        workspace_root: Option<PathBuf>,
        host_home_root: Option<PathBuf>,
        pool: deadpool_postgres::Pool,
        secret_master_key: ironclaw_secrets::SecretMaterial,
        process_local_resource_governor_singleton: bool,
    },
    #[cfg(feature = "libsql")]
    Libsql {
        db: Arc<libsql::Database>,
        path_or_url: String,
        auth_token: Option<ironclaw_secrets::SecretMaterial>,
        secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
        process_local_resource_governor_singleton: bool,
    },
    #[cfg(feature = "postgres")]
    Postgres {
        pool: deadpool_postgres::Pool,
        url: ironclaw_secrets::SecretMaterial,
        tls_options: PostgresPoolTlsOptions,
        secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
        process_local_resource_governor_singleton: bool,
    },
}

impl RebornBuildInput {
    /// Selected composition profile — a display/telemetry label. Behaviour
    /// comes from [`RebornBuildInput::deployment`].
    pub fn profile(&self) -> RebornCompositionProfile {
        self.deployment.profile()
    }

    /// The deployment axes this build assembles from.
    pub fn deployment(&self) -> &DeploymentConfig {
        &self.deployment
    }

    /// Replace the deployment this input was constructed with.
    ///
    /// Test-only: production builds the deployment at construction
    /// (`RebornBuildInput::new` takes it, and `local_runtime_build_input_with_options`
    /// supplies one built where the operator's yolo disclosure is known). This
    /// exists so tests can construct a deliberately mismatched
    /// deployment/storage pairing and drive the fail-closed guard in
    /// `build_reborn_services` — production behaviour, reached through a
    /// pairing production rejects.
    #[cfg(test)]
    pub(crate) fn with_deployment(mut self, deployment: DeploymentConfig) -> Self {
        self.deployment = deployment;
        self
    }

    /// Owner id (string form). Used by the assembled runtime to mint the
    /// `UserId` actor for inbound CLI messages.
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub(crate) fn has_nearai_mcp_bootstrap_config(&self) -> bool {
        self.nearai_mcp_bootstrap_config.is_some()
    }

    /// Override the owner id after construction.
    ///
    /// The WebChat v2 serve path uses this to pin the runtime owner to the
    /// authenticated WebUI user *after* the runtime input (and its host-access
    /// disclosure gate) has been built, so the turn-runner loop host reads
    /// thread context from the same `owners/<user>` subtree the v2 facade
    /// wrote to.
    pub fn with_owner_id(mut self, owner_id: impl Into<String>) -> Self {
        self.owner_id = owner_id.into();
        self
    }

    /// Override the local runtime tenant/agent identity used by command-style
    /// facades that need a surface context before a full runtime exists.
    pub fn with_local_runtime_identity(mut self, tenant_id: TenantId, agent_id: AgentId) -> Self {
        self.local_runtime_identity = Some(RebornLocalRuntimeIdentity {
            tenant_id,
            agent_id,
        });
        self
    }

    pub fn disabled(owner_id: impl Into<String>) -> Self {
        Self::new(
            DeploymentConfig::disabled(),
            owner_id,
            RebornStorageInput::Disabled,
        )
    }

    pub fn local_dev(owner_id: impl Into<String>, root: PathBuf) -> Self {
        Self::local_dev_from_deployment(DeploymentConfig::local_dev(), owner_id, root)
    }

    pub(crate) fn local_dev_with_profile(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        root: PathBuf,
    ) -> Self {
        Self::local_dev_from_deployment(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            root,
        )
    }

    /// Build a local-dev-storage-shaped input from an already-resolved
    /// deployment. The `debug_assert` is on the storage-shape **axis**, not on
    /// a list of profile names (§4.4).
    pub(crate) fn local_dev_from_deployment(
        deployment: DeploymentConfig,
        owner_id: impl Into<String>,
        root: PathBuf,
    ) -> Self {
        debug_assert!(deployment.uses_local_dev_storage_input());
        Self::new(
            deployment,
            owner_id,
            RebornStorageInput::LocalDev {
                root,
                workspace_root: None,
                host_home_root: None,
            },
        )
    }

    #[cfg(feature = "postgres")]
    pub fn hosted_single_tenant_postgres(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        root: PathBuf,
        pool: deadpool_postgres::Pool,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    ) -> Result<Self, RebornBuildError> {
        // The storage handle and the deployment must agree. Expressed as the
        // config's storage-shape axis rather than a profile-name comparison
        // (§4.4): a deployment that takes a hosted single-tenant pool is a
        // property of the deployment, not of its name.
        if DeploymentConfig::for_profile(profile, false).storage_shape()
            != crate::deployment::StorageShape::HostedSingleTenantPool
        {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "hosted single-tenant Postgres storage requires profile=hosted-single-tenant; got profile={profile}"
                ),
            });
        }
        Ok(Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::HostedSingleTenantPostgres {
                root,
                workspace_root: None,
                host_home_root: None,
                pool,
                secret_master_key,
                process_local_resource_governor_singleton: true,
            },
        ))
    }

    #[cfg(feature = "postgres")]
    pub fn hosted_single_tenant_postgres_from_config_and_env(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        root: PathBuf,
        config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
    ) -> Result<Self, RebornBuildError> {
        // The storage handle and the deployment must agree. Expressed as the
        // config's storage-shape axis rather than a profile-name comparison
        // (§4.4): a deployment that takes a hosted single-tenant pool is a
        // property of the deployment, not of its name.
        if DeploymentConfig::for_profile(profile, false).storage_shape()
            != crate::deployment::StorageShape::HostedSingleTenantPool
        {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "hosted single-tenant Postgres storage requires profile=hosted-single-tenant; got profile={profile}"
                ),
            });
        }
        let ResolvedPostgresStorage {
            pool,
            secret_master_key,
            process_local_resource_governor_singleton,
            ..
        } = resolve_postgres_storage_from_config_and_env(profile, config_file)?;
        Ok(Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::HostedSingleTenantPostgres {
                root,
                workspace_root: None,
                host_home_root: None,
                pool,
                secret_master_key,
                process_local_resource_governor_singleton,
            },
        ))
    }

    /// Open the hosted-single-tenant trigger access store from this build
    /// input's already-resolved PostgreSQL storage.
    #[cfg(feature = "postgres")]
    pub async fn open_hosted_single_tenant_trigger_access_store(
        &self,
    ) -> Result<Arc<dyn crate::LocalTriggerAccessStore>, crate::RebornLocalTriggerAccessStoreError>
    {
        let RebornStorageInput::HostedSingleTenantPostgres { pool, .. } = &self.storage else {
            return Err(crate::RebornLocalTriggerAccessStoreError::Backend(
                "hosted-single-tenant trigger access requires PostgreSQL-backed runtime storage"
                    .to_string(),
            ));
        };
        let filesystem = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(
            pool.clone(),
        ));
        filesystem.run_migrations().await.map_err(|error| {
            crate::RebornLocalTriggerAccessStoreError::Backend(error.to_string())
        })?;
        let scoped = crate::wrap_scoped(filesystem);
        Ok(Arc::new(
            crate::RebornFilesystemLocalTriggerAccessStore::new(scoped),
        ))
    }

    pub fn with_local_runtime_workspace_root(mut self, workspace_root: PathBuf) -> Self {
        match &mut self.storage {
            RebornStorageInput::LocalDev {
                workspace_root: root,
                ..
            } => {
                *root = Some(workspace_root);
            }
            #[cfg(feature = "postgres")]
            RebornStorageInput::HostedSingleTenantPostgres {
                workspace_root: root,
                ..
            } => {
                *root = Some(workspace_root);
            }
            _ => {}
        }
        self
    }

    pub fn with_local_dev_workspace_root(self, workspace_root: PathBuf) -> Self {
        self.with_local_runtime_workspace_root(workspace_root)
    }

    pub fn with_local_runtime_confirmed_host_home_root(mut self, host_home_root: PathBuf) -> Self {
        match &mut self.storage {
            RebornStorageInput::LocalDev {
                host_home_root: root,
                ..
            } => {
                *root = Some(host_home_root);
            }
            #[cfg(feature = "postgres")]
            RebornStorageInput::HostedSingleTenantPostgres {
                host_home_root: root,
                ..
            } => {
                *root = Some(host_home_root);
            }
            _ => {}
        }
        self
    }

    pub fn with_local_dev_confirmed_host_home_root(self, host_home_root: PathBuf) -> Self {
        self.with_local_runtime_confirmed_host_home_root(host_home_root)
    }

    pub fn requires_local_runtime_confirmed_host_home_root(&self) -> bool {
        self.runtime_policy.as_ref().is_some_and(|policy| {
            policy.filesystem_backend == FilesystemBackendKind::HostWorkspaceAndHome
        })
    }

    pub fn requires_local_dev_confirmed_host_home_root(&self) -> bool {
        self.requires_local_runtime_confirmed_host_home_root()
    }

    pub fn grants_trusted_laptop_access(&self) -> bool {
        self.runtime_policy.as_ref().is_some_and(|policy| {
            policy.filesystem_backend == FilesystemBackendKind::HostWorkspaceAndHome
                || policy.network_mode == NetworkMode::Direct
                || policy.secret_mode == SecretMode::InheritedEnv
        })
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
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::Libsql {
                db,
                path_or_url: path_or_url.into(),
                auth_token,
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton: true,
            },
        )
    }

    #[cfg(feature = "libsql")]
    pub fn libsql_with_resolved_secret_master_key(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        db: Arc<libsql::Database>,
        path_or_url: impl Into<String>,
        auth_token: Option<ironclaw_secrets::SecretMaterial>,
    ) -> Self {
        Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::Libsql {
                db,
                path_or_url: path_or_url.into(),
                auth_token,
                secret_master_key: None,
                process_local_resource_governor_singleton: true,
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
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::Postgres {
                pool,
                url,
                tls_options: PostgresPoolTlsOptions::default(),
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton: true,
            },
        )
    }

    #[cfg(feature = "postgres")]
    pub fn postgres_with_resolved_secret_master_key(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        pool: deadpool_postgres::Pool,
        url: ironclaw_secrets::SecretMaterial,
    ) -> Self {
        Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::Postgres {
                pool,
                url,
                tls_options: PostgresPoolTlsOptions::default(),
                secret_master_key: None,
                process_local_resource_governor_singleton: true,
            },
        )
    }

    #[cfg(feature = "postgres")]
    pub fn postgres_from_config_and_env(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
    ) -> Result<Self, RebornBuildError> {
        let ResolvedPostgresStorage {
            pool,
            url,
            tls_options,
            secret_master_key,
            process_local_resource_governor_singleton,
        } = resolve_postgres_storage_from_config_and_env(profile, config_file)?;
        let runtime_policy = resolve_production_runtime_policy(profile, config_file)?;
        let trust_policy = crate::builtin_first_party_trust_policy()?;

        Ok(Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::Postgres {
                pool,
                url,
                tls_options,
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton,
            },
        )
        .with_production_trust_policy(Arc::new(trust_policy))
        .with_runtime_policy(runtime_policy)
        .with_runtime_process_binding(RebornRuntimeProcessBinding::none()))
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

    pub fn with_turn_run_wake_notifier<T>(mut self, notifier: Arc<T>) -> Self
    where
        T: TurnRunWakeNotifier + 'static,
    {
        self.turn_run_wake_notifier = Some(notifier);
        self
    }

    pub fn with_turn_run_wake_notifier_dyn(
        mut self,
        notifier: Arc<dyn TurnRunWakeNotifier>,
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

    pub fn with_nearai_mcp_bootstrap_config(
        mut self,
        config: crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig,
    ) -> Self {
        self.nearai_mcp_bootstrap_config = Some(config);
        self
    }

    pub fn with_optional_nearai_mcp_bootstrap_config(
        mut self,
        config: Option<crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig>,
    ) -> Self {
        self.nearai_mcp_bootstrap_config = config;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_host_runtime_http_egress_for_test(
        mut self,
        egress: Option<HostRuntimeHttpEgressPort>,
    ) -> Self {
        self.host_runtime_http_egress_for_test = Some(egress);
        self
    }

    /// Override local-dev host HTTP egress for fixture recording and replay.
    ///
    /// This is compiled only for tests/test-support so Reborn QA harnesses can
    /// route host-mediated integration calls through trace record/replay
    /// adapters without changing production composition.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_network_http_egress_for_test(mut self, egress: Arc<dyn NetworkHttpEgress>) -> Self {
        self.network_http_egress_for_test = Some(egress);
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

    /// Record product/bootstrap-provided Google OAuth metadata on the build input.
    ///
    /// `RebornBuildInput` owns this composition seam until a settings-backed
    /// source exists.
    pub fn with_google_oauth_backend(mut self, config: OAuthClientConfig) -> Self {
        self.push_oauth_provider_config(google_provider_spec(), config);
        self
    }

    /// Record product/bootstrap-provided Notion MCP OAuth metadata on the build input.
    ///
    /// This keeps Notion OAuth in the Reborn product-auth provider path; callers
    /// that use dynamic client registration can pass the client metadata they
    /// registered for this host callback URL.
    pub fn with_notion_oauth_backend(mut self, config: OAuthClientConfig) -> Self {
        self.push_oauth_provider_config(notion_provider_spec(), config);
        self
    }

    /// Register the lazy Slack personal OAuth slot so the provider client
    /// fetches credentials from the setup service at request time rather than
    /// from env vars at startup.
    pub fn with_slack_personal_oauth_lazy(mut self, slot: SlackPersonalSetupServiceSlot) -> Self {
        self.slack_personal_oauth_lazy_slot = Some(slot);
        self
    }

    /// Record the build-time Slack host-beta wiring signal. The CLI `serve`
    /// path calls this before the composition build with whether it
    /// resolved a Slack host-beta
    /// config for this instance, so `provider_instance_readiness_map` can
    /// decide whether `slack_personal` needs a readiness-map entry.
    pub fn with_slack_host_beta_enabled(mut self, enabled: bool) -> Self {
        self.slack_host_beta_enabled = enabled;
        self
    }

    /// Record whether this instance resolved
    /// `IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI`. The CLI `serve`
    /// path passes the already-resolved slot's presence; it does not re-read
    /// the environment. See the field doc for why this is a separate signal
    /// from `with_slack_personal_oauth_lazy`.
    pub fn with_slack_personal_oauth_redirect_uri_configured(mut self, configured: bool) -> Self {
        self.slack_personal_oauth_redirect_uri_configured = configured;
        self
    }

    /// Enable Dynamic Client Registration for the bundled Notion MCP OAuth provider.
    ///
    /// Callers provide the public origin that serves the Reborn product-auth
    /// callback route. Local loopback HTTP origins are accepted; non-loopback
    /// deployments must use HTTPS.
    pub fn with_notion_dcr_oauth_backend(
        mut self,
        callback_origin: impl Into<String>,
        client_name: impl Into<String>,
    ) -> Result<Self, ironclaw_auth::AuthProductError> {
        self.push_oauth_dcr_provider_config(OAuthDcrProviderConfig {
            spec: notion_provider_spec(),
            callback_origin: callback_origin.into(),
            client_name: client_name.into(),
            account_label: CredentialAccountLabel::new("notion")?,
            scopes: Vec::new(),
        });
        Ok(self)
    }

    /// Set concurrency limits for the in-memory turn-state store.
    ///
    /// Called by `build_reborn_runtime` after mapping from `TurnRunnerSettings` so the
    /// factory can apply them when constructing the store. Callers should use
    /// `RebornRuntimeInput::with_runner_settings` rather than calling this directly.
    pub(crate) fn with_turn_state_store_limits(
        mut self,
        limits: InMemoryTurnStateStoreLimits,
    ) -> Self {
        self.turn_state_store_limits = limits;
        self
    }

    fn push_oauth_provider_config(
        &mut self,
        spec: HostOAuthProviderSpec,
        client: OAuthClientConfig,
    ) {
        if let Some(existing) = self
            .oauth_provider_configs
            .iter_mut()
            .find(|existing| existing.spec.provider_id == spec.provider_id)
        {
            existing.spec = spec;
            existing.client = client;
            return;
        }
        self.oauth_provider_configs
            .push(OAuthProviderBackendConfig { spec, client });
    }

    fn push_oauth_dcr_provider_config(&mut self, config: OAuthDcrProviderConfig) {
        if let Some(existing) = self
            .oauth_dcr_provider_configs
            .iter_mut()
            .find(|existing| existing.config.spec.provider_id == config.spec.provider_id)
        {
            existing.config = config;
            return;
        }
        self.oauth_dcr_provider_configs
            .push(OAuthDcrProviderBackendConfig { config });
    }

    fn new(
        deployment: DeploymentConfig,
        owner_id: impl Into<String>,
        storage: RebornStorageInput,
    ) -> Self {
        Self {
            deployment,
            owner_id: owner_id.into(),
            local_runtime_identity: None,
            storage,
            production_trust_policy: None,
            runtime_policy: None,
            turn_run_wake_notifier: None,
            runtime_process_binding: RebornRuntimeProcessBinding::default(),
            required_runtime_backends: Vec::new(),
            require_runtime_http_egress: false,
            require_wasm_credentials: false,
            #[cfg(test)]
            host_runtime_http_egress_for_test: None,
            #[cfg(any(test, feature = "test-support"))]
            network_http_egress_for_test: None,
            product_auth_ports: None,
            oauth_provider_configs: Vec::new(),
            oauth_dcr_provider_configs: Vec::new(),
            slack_personal_oauth_lazy_slot: None,
            slack_host_beta_enabled: false,
            slack_personal_oauth_redirect_uri_configured: false,
            nearai_mcp_bootstrap_config: None,
            turn_state_store_limits: InMemoryTurnStateStoreLimits::default(),
        }
    }
}

#[cfg(feature = "postgres")]
struct ResolvedPostgresStorage {
    pool: deadpool_postgres::Pool,
    url: ironclaw_secrets::SecretMaterial,
    tls_options: PostgresPoolTlsOptions,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
}

#[cfg(feature = "postgres")]
fn resolve_postgres_storage_from_config_and_env(
    profile: RebornCompositionProfile,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> Result<ResolvedPostgresStorage, RebornBuildError> {
    let storage = config_file
        .and_then(|file| file.storage.as_ref())
        .ok_or_else(|| RebornBuildError::InvalidConfig {
            reason: format!(
                "profile={profile} requires [storage] backend = \"postgres\" with url_env naming \
                 an environment variable such as {DEFAULT_REBORN_POSTGRES_URL_ENV}"
            ),
        })?;
    match storage.backend.as_ref() {
        Some(StorageBackend::Postgres) => {}
        Some(StorageBackend::Unknown(backend)) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "PostgreSQL-backed Reborn storage supports only [storage].backend = \"postgres\" in this slice; got `{backend}`"
                ),
            });
        }
        None => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("profile={profile} requires [storage].backend = \"postgres\""),
            });
        }
    }
    let url_env = storage
        .url_env
        .as_deref()
        .unwrap_or(DEFAULT_REBORN_POSTGRES_URL_ENV);
    let secret_master_key_env = storage
        .secret_master_key_env
        .as_deref()
        .unwrap_or(DEFAULT_REBORN_SECRET_MASTER_KEY_ENV);
    let database_url =
        required_production_url_env(url_env, "Reborn PostgreSQL URL", "storage.url_env")?;
    let secret_master_key = required_production_key_env(
        secret_master_key_env,
        "Reborn secret master key",
        "storage.secret_master_key_env",
    )?;
    let process_local_resource_governor_singleton =
        require_postgres_resource_governor_singleton_env()?;
    let (pool_max_size, pool_max_size_source) =
        resolve_postgres_pool_max_size(storage.pool_max_size)?;
    tracing::debug!(
        %profile,
        pool_max_size,
        pool_max_size_source,
        "resolved Reborn PostgreSQL pool size"
    );
    let tls_options = postgres_pool_tls_options_from_env()?;
    let pool = ironclaw_reborn_event_store::open_postgres_pool_with_tls_options(
        database_url.clone(),
        pool_max_size,
        tls_options,
    )?;

    Ok(ResolvedPostgresStorage {
        pool,
        url: database_url,
        tls_options,
        secret_master_key,
        process_local_resource_governor_singleton,
    })
}

#[cfg(feature = "postgres")]
fn resolve_production_runtime_policy(
    profile: RebornCompositionProfile,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> Result<EffectiveRuntimePolicy, RebornBuildError> {
    let policy = config_file
        .and_then(|file| file.policy.as_ref())
        .ok_or_else(|| RebornBuildError::InvalidConfig {
            reason: format!(
                "profile={profile} requires [policy].deployment_mode and [policy].default_profile"
            ),
        })?;
    let deployment_mode =
        policy
            .deployment_mode
            .as_deref()
            .ok_or_else(|| RebornBuildError::InvalidConfig {
                reason: format!("profile={profile} requires [policy].deployment_mode"),
            })?;
    let default_profile =
        policy
            .default_profile
            .as_deref()
            .ok_or_else(|| RebornBuildError::InvalidConfig {
                reason: format!("profile={profile} requires [policy].default_profile"),
            })?;
    let deployment = DeploymentMode::from_str(deployment_mode).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("invalid [policy].deployment_mode `{deployment_mode}`: {error}"),
        }
    })?;
    let requested_profile = RuntimeProfile::from_str(default_profile).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("invalid [policy].default_profile `{default_profile}`: {error}"),
        }
    })?;
    crate::resolve_runtime_policy(crate::RuntimePolicyResolveRequest::new(
        deployment,
        requested_profile,
    ))
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!(
            "failed to resolve runtime policy for deployment_mode={deployment_mode} \
             default_profile={default_profile}: {error}"
        ),
    })
}

#[cfg(feature = "postgres")]
fn resolve_postgres_pool_max_size(
    configured: Option<usize>,
) -> Result<(usize, &'static str), RebornBuildError> {
    match std::env::var(REBORN_POSTGRES_POOL_MAX_SIZE_ENV) {
        Ok(raw) => {
            let trimmed = raw.trim();
            let parsed = trimmed
                .parse::<usize>()
                .map_err(|_| RebornBuildError::InvalidConfig {
                    reason: format!(
                        "{REBORN_POSTGRES_POOL_MAX_SIZE_ENV} must be a positive integer"
                    ),
                })?;
            if parsed == 0 {
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!("{REBORN_POSTGRES_POOL_MAX_SIZE_ENV} must be greater than 0"),
                });
            }
            Ok((parsed, "env"))
        }
        Err(std::env::VarError::NotPresent) => Ok(configured.map_or(
            (
                ironclaw_reborn_event_store::DEFAULT_POSTGRES_POOL_MAX_SIZE,
                "default",
            ),
            |value| (value, "config"),
        )),
        Err(std::env::VarError::NotUnicode(_)) => Err(RebornBuildError::InvalidConfig {
            reason: format!("{REBORN_POSTGRES_POOL_MAX_SIZE_ENV} must be valid Unicode"),
        }),
    }
}

#[cfg(feature = "postgres")]
fn required_production_url_env(
    env_name: &str,
    description: &str,
    config_field: &str,
) -> Result<SecretString, RebornBuildError> {
    let value = std::env::var(env_name).map_err(|_| RebornBuildError::InvalidConfig {
        reason: format!(
            "{env_name} must be set to the {description}; config.toml may only name this env var via [{config_field}], never contain the secret value"
        ),
    })?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RebornBuildError::InvalidConfig {
            reason: format!("{env_name} must not be empty"),
        });
    }
    Ok(SecretString::from(trimmed.to_string()))
}

#[cfg(feature = "postgres")]
fn required_production_key_env(
    env_name: &str,
    description: &str,
    config_field: &str,
) -> Result<SecretString, RebornBuildError> {
    let value = std::env::var(env_name).map_err(|_| RebornBuildError::InvalidConfig {
        reason: format!(
            "{env_name} must be set to the {description}; config.toml may only name this env var via [{config_field}], never contain the secret value"
        ),
    })?;
    if value.is_empty() {
        return Err(RebornBuildError::InvalidConfig {
            reason: format!("{env_name} must not be empty"),
        });
    }
    Ok(SecretString::from(value))
}

#[cfg(feature = "postgres")]
fn require_postgres_resource_governor_singleton_env() -> Result<bool, RebornBuildError> {
    match std::env::var(REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV) {
        Ok(value) => match parse_bool_opt_in(&value) {
            Some(true) => Ok(true),
            Some(false) => Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "{REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV} must be true when this process is the singleton or elected resource-governor authority for the shared Postgres database"
                ),
            }),
            None => Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "{REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV} must be one of true, false, 1, 0, yes, no, on, or off"
                ),
            }),
        },
        Err(std::env::VarError::NotPresent) => Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "{REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV} must be set to true when this process is the singleton or elected resource-governor authority for the shared Postgres database"
            ),
        }),
        Err(std::env::VarError::NotUnicode(_)) => Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "{REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV} must be valid UTF-8"
            ),
        }),
    }
}

#[cfg(feature = "postgres")]
fn postgres_pool_tls_options_from_env() -> Result<PostgresPoolTlsOptions, RebornBuildError> {
    let ssl_mode_override = match std::env::var(DATABASE_SSLMODE_ENV) {
        Ok(value) if value.trim().is_empty() => None,
        Ok(value) => Some(
            value
                .trim()
                .parse::<RebornPostgresSslMode>()
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!("{DATABASE_SSLMODE_ENV}: {error}"),
                })?,
        ),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("{DATABASE_SSLMODE_ENV} must be valid UTF-8"),
            });
        }
    };
    let allow_remote_cleartext = match std::env::var(ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV) {
        Ok(value) => parse_bool_opt_in(&value).ok_or_else(|| {
            RebornBuildError::InvalidConfig {
                reason: format!(
                    "{ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV} must be one of true, false, 1, 0, yes, no, on, or off"
                ),
            }
        })?,
        Err(std::env::VarError::NotPresent) => false,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("{ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV} must be valid UTF-8"),
            });
        }
    };

    Ok(PostgresPoolTlsOptions {
        ssl_mode_override,
        allow_remote_cleartext,
    })
}

#[cfg(feature = "postgres")]
fn parse_bool_opt_in(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Some(false),
        "1" | "true" | "yes" | "on" => Some(true),
        _ => None,
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
