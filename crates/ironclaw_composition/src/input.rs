use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use ironclaw_auth::{AuthProductError, OAuthClientId, OAuthRedirectUri};
use ironclaw_host_api::runtime_policy::ProcessBackendKind;
use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};
use ironclaw_host_api::runtime_policy::{
    EffectiveRuntimePolicy, FilesystemBackendKind, NetworkMode, SecretMode,
};
use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_host_runtime::TenantSandboxProcessPort;
#[cfg(any(test, feature = "test-support"))]
use ironclaw_network::NetworkHttpEgress;
use ironclaw_trust::HostTrustPolicy;
use ironclaw_turns::{TurnRunWakeNotifier, TurnStateStoreLimits};
use secrecy::SecretString;

use ironclaw_config::StorageBackend;
use ironclaw_event_store::{IronClawPostgresSslMode, PostgresPoolTlsOptions};

use crate::IronClawBuildError;
use crate::deployment::DeploymentConfig;
use crate::{IronClawCompositionProfile, IronClawProductAuthServicePorts};

const DEFAULT_POSTGRES_URL_ENV: &str = "IRONCLAW_POSTGRES_URL";
const DEFAULT_SECRET_MASTER_KEY_ENV: &str = "IRONCLAW_SECRET_MASTER_KEY";
const POSTGRES_POOL_MAX_SIZE_ENV: &str = "IRONCLAW_POSTGRES_POOL_MAX_SIZE";
const POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV: &str =
    "IRONCLAW_POSTGRES_RESOURCE_GOVERNOR_SINGLETON";
const DATABASE_SSLMODE_ENV: &str = "DATABASE_SSLMODE";
const ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV: &str = "IRONCLAW_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT";

/// Composition-time OAuth client metadata.
///
/// `IronClawBuildInput` owns this seam for product/bootstrap-provided values
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

/// Deployment OAuth client material for one vendor id. The vendor's recipe
/// (from its manifest) names the client-credential handles; this config
/// supplies their values.
#[derive(Debug, Clone)]
pub(crate) struct OAuthProviderBackendConfig {
    pub(crate) vendor: String,
    pub(crate) client: OAuthClientConfig,
}

/// The public origin serving the static vendor OAuth callback routes —
/// enables dynamic client registration (and the engine callback base) for
/// vendors whose recipes carry no deployment client credentials.
#[derive(Debug, Clone)]
pub(crate) struct OAuthDcrCallbackConfig {
    pub(crate) callback_origin: String,
}

#[derive(Clone, Debug, Default)]
pub enum IronClawRuntimeProcessBinding {
    #[default]
    None,
    TenantSandbox {
        process_port: Arc<TenantSandboxProcessPort>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IronClawRuntimeProcessBindingError {
    MissingTenantSandboxProcessPort,
    UnexpectedTenantSandboxProcessPort { process_backend: ProcessBackendKind },
}

impl std::fmt::Display for IronClawRuntimeProcessBindingError {
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

impl IronClawRuntimeProcessBinding {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn tenant_sandbox(process_port: Arc<TenantSandboxProcessPort>) -> Self {
        Self::TenantSandbox { process_port }
    }

    pub(crate) fn validate_for_production_policy(
        &self,
        runtime_policy: &EffectiveRuntimePolicy,
    ) -> Result<(), IronClawRuntimeProcessBindingError> {
        match (runtime_policy.process_backend, self) {
            (
                ProcessBackendKind::TenantSandbox,
                IronClawRuntimeProcessBinding::TenantSandbox { .. },
            ) => Ok(()),
            (ProcessBackendKind::TenantSandbox, IronClawRuntimeProcessBinding::None) => {
                Err(IronClawRuntimeProcessBindingError::MissingTenantSandboxProcessPort)
            }
            (_, IronClawRuntimeProcessBinding::TenantSandbox { .. }) => Err(
                IronClawRuntimeProcessBindingError::UnexpectedTenantSandboxProcessPort {
                    process_backend: runtime_policy.process_backend,
                },
            ),
            (_, IronClawRuntimeProcessBinding::None) => Ok(()),
        }
    }
}

pub struct IronClawBuildInput {
    /// The deployment this build assembles, as data (§4.4/§5.6). Carries the
    /// substrate, traffic, readiness, and storage-shape axes every consumer
    /// reads instead of re-deriving them from a profile name.
    ///
    /// The **resolved** runtime policy rides `runtime_policy`, not this value:
    /// `new` builds the config without a yolo host-access disclosure (it is not
    /// known at construction), so callers that hold the operator's confirmation
    /// install the accurate config through
    /// [`IronClawBuildInput::with_deployment`] — `local_runtime_build_input_with_options`
    /// is the one that does.
    pub(crate) deployment: DeploymentConfig,
    pub(crate) owner_id: String,
    pub(crate) local_runtime_identity: Option<IronClawLocalRuntimeIdentity>,
    pub(crate) storage: IronClawStorageInput,
    pub(crate) production_trust_policy: Option<Arc<HostTrustPolicy>>,
    pub(crate) runtime_policy: Option<EffectiveRuntimePolicy>,
    pub(crate) turn_run_wake_notifier: Option<Arc<dyn TurnRunWakeNotifier>>,
    pub(crate) runtime_process_binding: IronClawRuntimeProcessBinding,
    pub(crate) required_runtime_backends: Vec<ironclaw_host_api::RuntimeKind>,
    pub(crate) require_runtime_http_egress: bool,
    pub(crate) require_wasm_credentials: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) network_http_egress_for_test: Option<Arc<dyn NetworkHttpEgress>>,
    /// Test-support only: stamp filesystem-discovered extension packages as
    /// `HostBundled` so integration fixtures that model host-bundled
    /// extensions (the §8 invented-vendor fixture) may assert
    /// first-party trust. Production discovery always stamps
    /// `InstalledLocal` (#5459).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) trust_fixture_extensions_for_test: bool,
    pub(crate) product_auth_ports: Option<IronClawProductAuthServicePorts>,
    pub(crate) oauth_provider_configs: Vec<OAuthProviderBackendConfig>,
    pub(crate) oauth_dcr_callback: Option<OAuthDcrCallbackConfig>,
    pub(crate) nearai_mcp_bootstrap_config:
        Option<crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig>,
    /// `first_party`-runtime extension factories the binary assembles
    /// (extension-runtime P2). Empty until concrete extension crates extract
    /// in P6; integration tests register the invented-vendor fixture factory
    /// here.
    pub(crate) native_extension_factories:
        Vec<std::sync::Arc<dyn ironclaw_extension_host::NativeExtensionFactory>>,
    /// Channel-adapter bindings + extras the binary assembles for channel
    /// extensions whose runtime is not `first_party` (extension-runtime
    /// DEL-7): the generic loader binds the adapter at activation and the
    /// channel host assembly consumes the extras. Composition never names a
    /// concrete extension crate.
    pub(crate) channel_extension_bindings: Vec<ChannelExtensionBinding>,
    /// Concurrency limits applied to the in-memory turn-state store.
    /// Defaults to no limits (all caps `None` / unlimited).
    pub(crate) turn_state_store_limits: TurnStateStoreLimits,
    /// Binary-assembled account-setup declarations (extension-runtime §5.5):
    /// per-extension activation gates and connect-strategy presentation.
    /// `WebGeneratedCode` declarations additionally get a generic pairing
    /// service composed over the durable identity/pairing stores.
    pub(crate) account_setup_descriptors:
        Vec<ironclaw_product_workflow::ExtensionAccountSetupDescriptor>,
}

/// One channel extension's binary-assembled vendor binding
/// (extension-runtime DEL-7): the adapter linked into this deployment plus
/// the composition extras the generic channel host consumes.
/// Supplied through [`IronClawBuildInput::with_channel_extension_bindings`] by
/// the assembling binary — composition itself never names a concrete
/// extension crate.
#[derive(Clone)]
pub struct ChannelExtensionBinding {
    /// The extension id the manifest declares (also the adapter id).
    pub extension_id: String,
    /// The channel adapter implementation linked into the deployment.
    pub adapter: std::sync::Arc<dyn ironclaw_product_adapters::ChannelAdapter>,
    /// Protocol-specific inbound payload reclassification (gate-resolution
    /// replies), registered on the channel host assembly.
    pub inbound_payload_classifier:
        Option<std::sync::Arc<crate::extension_host::extension_ingress::InboundPayloadClassifier>>,
    /// The vendor half of the preference-target codec, consumed by the
    /// generic outbound-target provider and triggered-delivery hook.
    pub preference_target_codec:
        Option<std::sync::Arc<dyn ironclaw_product_workflow::PreferenceTargetCodec>>,
}

#[derive(Clone, Debug)]
pub(crate) struct IronClawLocalRuntimeIdentity {
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
}

pub(crate) enum IronClawStorageInput {
    Disabled,
    LocalDev {
        root: PathBuf,
        workspace_root: Option<PathBuf>,
        host_home_root: Option<PathBuf>,
    },
    HostedSingleTenantPostgres {
        root: PathBuf,
        workspace_root: Option<PathBuf>,
        host_home_root: Option<PathBuf>,
        pool: deadpool_postgres::Pool,
        secret_master_key: ironclaw_secrets::SecretMaterial,
        process_local_resource_governor_singleton: bool,
    },
    Libsql {
        db: Arc<libsql::Database>,
        path_or_url: String,
        auth_token: Option<ironclaw_secrets::SecretMaterial>,
        secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
        process_local_resource_governor_singleton: bool,
    },
    Postgres {
        pool: deadpool_postgres::Pool,
        url: ironclaw_secrets::SecretMaterial,
        tls_options: PostgresPoolTlsOptions,
        secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
        process_local_resource_governor_singleton: bool,
    },
}

impl IronClawBuildInput {
    /// Selected composition profile — a display/telemetry label. Behaviour
    /// comes from [`IronClawBuildInput::deployment`].
    pub fn profile(&self) -> IronClawCompositionProfile {
        self.deployment.profile()
    }

    /// The deployment axes this build assembles from.
    pub fn deployment(&self) -> &DeploymentConfig {
        &self.deployment
    }

    /// Replace the deployment this input was constructed with.
    ///
    /// Test-only: production builds the deployment at construction
    /// (`IronClawBuildInput::new` takes it, and `local_runtime_build_input_with_options`
    /// supplies one built where the operator's yolo disclosure is known). This
    /// exists so tests can construct a deliberately mismatched
    /// deployment/storage pairing and drive the fail-closed guard in
    /// `build_ironclaw_services` — production behaviour, reached through a
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
        self.local_runtime_identity = Some(IronClawLocalRuntimeIdentity {
            tenant_id,
            agent_id,
        });
        self
    }

    pub fn disabled(owner_id: impl Into<String>) -> Self {
        Self::new(
            DeploymentConfig::disabled(),
            owner_id,
            IronClawStorageInput::Disabled,
        )
    }

    pub fn local_dev(owner_id: impl Into<String>, root: PathBuf) -> Self {
        Self::local_dev_from_deployment(DeploymentConfig::local_dev(), owner_id, root)
    }

    pub(crate) fn local_dev_with_profile(
        profile: IronClawCompositionProfile,
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
            IronClawStorageInput::LocalDev {
                root,
                workspace_root: None,
                host_home_root: None,
            },
        )
    }

    pub fn hosted_single_tenant_postgres(
        profile: IronClawCompositionProfile,
        owner_id: impl Into<String>,
        root: PathBuf,
        pool: deadpool_postgres::Pool,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    ) -> Result<Self, IronClawBuildError> {
        // The storage handle and the deployment must agree. Expressed as the
        // config's storage-shape axis rather than a profile-name comparison
        // (§4.4): a deployment that takes a hosted single-tenant pool is a
        // property of the deployment, not of its name.
        if DeploymentConfig::for_profile(profile, false).storage_shape()
            != crate::deployment::StorageShape::HostedSingleTenantPool
        {
            return Err(IronClawBuildError::InvalidConfig {
                reason: format!(
                    "hosted single-tenant Postgres storage requires profile=hosted-single-tenant; got profile={profile}"
                ),
            });
        }
        Ok(Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            IronClawStorageInput::HostedSingleTenantPostgres {
                root,
                workspace_root: None,
                host_home_root: None,
                pool,
                secret_master_key,
                process_local_resource_governor_singleton: true,
            },
        ))
    }

    pub fn hosted_single_tenant_postgres_from_config_and_env(
        profile: IronClawCompositionProfile,
        owner_id: impl Into<String>,
        root: PathBuf,
        config_file: Option<&ironclaw_config::IronClawConfigFile>,
    ) -> Result<Self, IronClawBuildError> {
        // The storage handle and the deployment must agree. Expressed as the
        // config's storage-shape axis rather than a profile-name comparison
        // (§4.4): a deployment that takes a hosted single-tenant pool is a
        // property of the deployment, not of its name.
        if DeploymentConfig::for_profile(profile, false).storage_shape()
            != crate::deployment::StorageShape::HostedSingleTenantPool
        {
            return Err(IronClawBuildError::InvalidConfig {
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
            IronClawStorageInput::HostedSingleTenantPostgres {
                root,
                workspace_root: None,
                host_home_root: None,
                pool,
                secret_master_key,
                process_local_resource_governor_singleton,
            },
        ))
    }

    pub fn with_local_runtime_workspace_root(mut self, workspace_root: PathBuf) -> Self {
        match &mut self.storage {
            IronClawStorageInput::LocalDev {
                workspace_root: root,
                ..
            } => {
                *root = Some(workspace_root);
            }
            IronClawStorageInput::HostedSingleTenantPostgres {
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
            IronClawStorageInput::LocalDev {
                host_home_root: root,
                ..
            } => {
                *root = Some(host_home_root);
            }
            IronClawStorageInput::HostedSingleTenantPostgres {
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

    pub fn libsql(
        profile: IronClawCompositionProfile,
        owner_id: impl Into<String>,
        db: Arc<libsql::Database>,
        path_or_url: impl Into<String>,
        auth_token: Option<ironclaw_secrets::SecretMaterial>,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    ) -> Self {
        Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            IronClawStorageInput::Libsql {
                db,
                path_or_url: path_or_url.into(),
                auth_token,
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton: true,
            },
        )
    }

    pub fn libsql_with_resolved_secret_master_key(
        profile: IronClawCompositionProfile,
        owner_id: impl Into<String>,
        db: Arc<libsql::Database>,
        path_or_url: impl Into<String>,
        auth_token: Option<ironclaw_secrets::SecretMaterial>,
    ) -> Self {
        Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            IronClawStorageInput::Libsql {
                db,
                path_or_url: path_or_url.into(),
                auth_token,
                secret_master_key: None,
                process_local_resource_governor_singleton: true,
            },
        )
    }

    pub fn postgres(
        profile: IronClawCompositionProfile,
        owner_id: impl Into<String>,
        pool: deadpool_postgres::Pool,
        url: ironclaw_secrets::SecretMaterial,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    ) -> Self {
        Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            IronClawStorageInput::Postgres {
                pool,
                url,
                tls_options: PostgresPoolTlsOptions::default(),
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton: true,
            },
        )
    }

    pub fn postgres_with_resolved_secret_master_key(
        profile: IronClawCompositionProfile,
        owner_id: impl Into<String>,
        pool: deadpool_postgres::Pool,
        url: ironclaw_secrets::SecretMaterial,
    ) -> Self {
        Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            IronClawStorageInput::Postgres {
                pool,
                url,
                tls_options: PostgresPoolTlsOptions::default(),
                secret_master_key: None,
                process_local_resource_governor_singleton: true,
            },
        )
    }

    pub fn postgres_from_config_and_env(
        profile: IronClawCompositionProfile,
        owner_id: impl Into<String>,
        config_file: Option<&ironclaw_config::IronClawConfigFile>,
    ) -> Result<Self, IronClawBuildError> {
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
            IronClawStorageInput::Postgres {
                pool,
                url,
                tls_options,
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton,
            },
        )
        .with_production_trust_policy(Arc::new(trust_policy))
        .with_runtime_policy(runtime_policy)
        .with_runtime_process_binding(IronClawRuntimeProcessBinding::none()))
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

    pub fn with_runtime_process_binding(mut self, binding: IronClawRuntimeProcessBinding) -> Self {
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

    pub fn with_native_extension_factories(
        mut self,
        factories: Vec<std::sync::Arc<dyn ironclaw_extension_host::NativeExtensionFactory>>,
    ) -> Self {
        self.native_extension_factories = factories;
        self
    }

    /// Supply the binary-assembled channel-adapter bindings for channel
    /// extensions whose runtime is not `first_party` (extension-runtime
    /// DEL-7): the generic loader binds each adapter at activation, and the
    /// channel host assembly registers the accompanying extras (gate-reply
    /// classifier, preference-target codec).
    pub fn with_channel_extension_bindings(
        mut self,
        bindings: Vec<ChannelExtensionBinding>,
    ) -> Self {
        self.channel_extension_bindings = bindings;
        self
    }

    /// Binary-assembled account-setup descriptors (see the field doc).
    pub fn with_account_setup_descriptors(
        mut self,
        descriptors: Vec<ironclaw_product_workflow::ExtensionAccountSetupDescriptor>,
    ) -> Self {
        self.account_setup_descriptors = descriptors;
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

    /// Override local-dev host HTTP egress for fixture recording and replay.
    ///
    /// This is compiled only for tests/test-support so IronClaw QA harnesses can
    /// route host-mediated integration calls through trace record/replay
    /// adapters without changing production composition.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_network_http_egress_for_test(mut self, egress: Arc<dyn NetworkHttpEgress>) -> Self {
        self.network_http_egress_for_test = Some(egress);
        self
    }

    /// Trust filesystem-discovered fixture extensions as host-bundled
    /// (first-party-eligible). Test-support only; see the field doc.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_trusted_fixture_extensions_for_test(mut self) -> Self {
        self.trust_fixture_extensions_for_test = true;
        self
    }

    /// Inject IronClaw-native product-auth service ports.
    ///
    /// Production callers should provide durable implementations here. The
    /// composition root attaches the turn-continuation dispatcher after it has
    /// composed the profile's [`ironclaw_turns::TurnCoordinator`], so OAuth
    /// continuations cannot accidentally bypass the active coordinator.
    pub fn with_product_auth_ports(mut self, ports: IronClawProductAuthServicePorts) -> Self {
        self.product_auth_ports = Some(ports);
        self
    }

    /// Record deployment OAuth client material for one vendor id. The vendor's
    /// manifest recipe names the client-credential handles these values fill.
    ///
    /// `IronClawBuildInput` owns this composition seam until a settings-backed
    /// source exists.
    pub fn with_vendor_oauth_client(
        mut self,
        vendor: impl Into<String>,
        config: OAuthClientConfig,
    ) -> Self {
        self.push_oauth_provider_config(vendor.into(), config);
        self
    }

    /// Record the public origin serving the vendor OAuth callback routes.
    /// Enables the engine's dynamic client registration (RFC 7591) for
    /// recipes without deployment client credentials, and anchors the static
    /// vendor callback base. Local loopback HTTP origins are accepted;
    /// non-loopback deployments must use HTTPS.
    pub fn with_dcr_oauth_callback(
        mut self,
        callback_origin: impl Into<String>,
    ) -> Result<Self, ironclaw_auth::AuthProductError> {
        let callback_origin = callback_origin.into();
        validate_dcr_callback_origin(&callback_origin)?;
        self.oauth_dcr_callback = Some(OAuthDcrCallbackConfig { callback_origin });
        Ok(self)
    }

    /// Set concurrency limits for the in-memory turn-state store.
    ///
    /// Called by `build_ironclaw_runtime` after mapping from `TurnRunnerSettings` so the
    /// factory can apply them when constructing the store. Callers should use
    /// `IronClawRuntimeInput::with_runner_settings` rather than calling this directly.
    pub(crate) fn with_turn_state_store_limits(mut self, limits: TurnStateStoreLimits) -> Self {
        self.turn_state_store_limits = limits;
        self
    }

    fn push_oauth_provider_config(&mut self, vendor: String, client: OAuthClientConfig) {
        if let Some(existing) = self
            .oauth_provider_configs
            .iter_mut()
            .find(|existing| existing.vendor == vendor)
        {
            existing.client = client;
            return;
        }
        self.oauth_provider_configs
            .push(OAuthProviderBackendConfig { vendor, client });
    }

    fn new(
        deployment: DeploymentConfig,
        owner_id: impl Into<String>,
        storage: IronClawStorageInput,
    ) -> Self {
        Self {
            deployment,
            owner_id: owner_id.into(),
            local_runtime_identity: None,
            storage,
            production_trust_policy: None,
            runtime_policy: None,
            turn_run_wake_notifier: None,
            runtime_process_binding: IronClawRuntimeProcessBinding::default(),
            required_runtime_backends: Vec::new(),
            require_runtime_http_egress: false,
            require_wasm_credentials: false,
            #[cfg(any(test, feature = "test-support"))]
            network_http_egress_for_test: None,
            #[cfg(any(test, feature = "test-support"))]
            trust_fixture_extensions_for_test: false,
            product_auth_ports: None,
            oauth_provider_configs: Vec::new(),
            oauth_dcr_callback: None,
            nearai_mcp_bootstrap_config: None,
            native_extension_factories: Vec::new(),
            channel_extension_bindings: Vec::new(),
            turn_state_store_limits: TurnStateStoreLimits::default(),
            account_setup_descriptors: Vec::new(),
        }
    }
}

struct ResolvedPostgresStorage {
    pool: deadpool_postgres::Pool,
    url: ironclaw_secrets::SecretMaterial,
    tls_options: PostgresPoolTlsOptions,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
}

fn resolve_postgres_storage_from_config_and_env(
    profile: IronClawCompositionProfile,
    config_file: Option<&ironclaw_config::IronClawConfigFile>,
) -> Result<ResolvedPostgresStorage, IronClawBuildError> {
    let storage = config_file
        .and_then(|file| file.storage.as_ref())
        .ok_or_else(|| IronClawBuildError::InvalidConfig {
            reason: format!(
                "profile={profile} requires [storage] backend = \"postgres\" with url_env naming \
                 an environment variable such as {DEFAULT_POSTGRES_URL_ENV}"
            ),
        })?;
    match storage.backend.as_ref() {
        Some(StorageBackend::Postgres) => {}
        Some(StorageBackend::Unknown(backend)) => {
            return Err(IronClawBuildError::InvalidConfig {
                reason: format!(
                    "PostgreSQL-backed IronClaw storage supports only [storage].backend = \"postgres\" in this slice; got `{backend}`"
                ),
            });
        }
        None => {
            return Err(IronClawBuildError::InvalidConfig {
                reason: format!("profile={profile} requires [storage].backend = \"postgres\""),
            });
        }
    }
    let url_env = storage
        .url_env
        .as_deref()
        .unwrap_or(DEFAULT_POSTGRES_URL_ENV);
    let secret_master_key_env = storage
        .secret_master_key_env
        .as_deref()
        .unwrap_or(DEFAULT_SECRET_MASTER_KEY_ENV);
    let database_url =
        required_production_url_env(url_env, "IronClaw PostgreSQL URL", "storage.url_env")?;
    let secret_master_key = required_production_key_env(
        secret_master_key_env,
        "IronClaw secret master key",
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
        "resolved IronClaw PostgreSQL pool size"
    );
    let tls_options = postgres_pool_tls_options_from_env()?;
    let pool = ironclaw_event_store::open_postgres_pool_with_tls_options(
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

fn resolve_production_runtime_policy(
    profile: IronClawCompositionProfile,
    config_file: Option<&ironclaw_config::IronClawConfigFile>,
) -> Result<EffectiveRuntimePolicy, IronClawBuildError> {
    let policy = config_file
        .and_then(|file| file.policy.as_ref())
        .ok_or_else(|| IronClawBuildError::InvalidConfig {
            reason: format!(
                "profile={profile} requires [policy].deployment_mode and [policy].default_profile"
            ),
        })?;
    let deployment_mode =
        policy
            .deployment_mode
            .as_deref()
            .ok_or_else(|| IronClawBuildError::InvalidConfig {
                reason: format!("profile={profile} requires [policy].deployment_mode"),
            })?;
    let default_profile =
        policy
            .default_profile
            .as_deref()
            .ok_or_else(|| IronClawBuildError::InvalidConfig {
                reason: format!("profile={profile} requires [policy].default_profile"),
            })?;
    let deployment = DeploymentMode::from_str(deployment_mode).map_err(|error| {
        IronClawBuildError::InvalidConfig {
            reason: format!("invalid [policy].deployment_mode `{deployment_mode}`: {error}"),
        }
    })?;
    let requested_profile = RuntimeProfile::from_str(default_profile).map_err(|error| {
        IronClawBuildError::InvalidConfig {
            reason: format!("invalid [policy].default_profile `{default_profile}`: {error}"),
        }
    })?;
    crate::resolve_runtime_policy(crate::RuntimePolicyResolveRequest::new(
        deployment,
        requested_profile,
    ))
    .map_err(|error| IronClawBuildError::InvalidConfig {
        reason: format!(
            "failed to resolve runtime policy for deployment_mode={deployment_mode} \
             default_profile={default_profile}: {error}"
        ),
    })
}

fn resolve_postgres_pool_max_size(
    configured: Option<usize>,
) -> Result<(usize, &'static str), IronClawBuildError> {
    match env_var_with_legacy(POSTGRES_POOL_MAX_SIZE_ENV) {
        Ok(raw) => {
            let trimmed = raw.trim();
            let parsed =
                trimmed
                    .parse::<usize>()
                    .map_err(|_| IronClawBuildError::InvalidConfig {
                        reason: format!("{POSTGRES_POOL_MAX_SIZE_ENV} must be a positive integer"),
                    })?;
            if parsed == 0 {
                return Err(IronClawBuildError::InvalidConfig {
                    reason: format!("{POSTGRES_POOL_MAX_SIZE_ENV} must be greater than 0"),
                });
            }
            Ok((parsed, "env"))
        }
        Err(std::env::VarError::NotPresent) => Ok(configured.map_or(
            (
                ironclaw_event_store::DEFAULT_POSTGRES_POOL_MAX_SIZE,
                "default",
            ),
            |value| (value, "config"),
        )),
        Err(std::env::VarError::NotUnicode(_)) => Err(IronClawBuildError::InvalidConfig {
            reason: format!("{POSTGRES_POOL_MAX_SIZE_ENV} must be valid Unicode"),
        }),
    }
}

fn required_production_url_env(
    env_name: &str,
    description: &str,
    config_field: &str,
) -> Result<SecretString, IronClawBuildError> {
    let value = env_var_with_legacy(env_name).map_err(|_| IronClawBuildError::InvalidConfig {
        reason: format!(
            "{env_name} must be set to the {description}; config.toml may only name this env var via [{config_field}], never contain the secret value"
        ),
    })?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(IronClawBuildError::InvalidConfig {
            reason: format!("{env_name} must not be empty"),
        });
    }
    Ok(SecretString::from(trimmed.to_string()))
}

fn required_production_key_env(
    env_name: &str,
    description: &str,
    config_field: &str,
) -> Result<SecretString, IronClawBuildError> {
    let value = env_var_with_legacy(env_name).map_err(|_| IronClawBuildError::InvalidConfig {
        reason: format!(
            "{env_name} must be set to the {description}; config.toml may only name this env var via [{config_field}], never contain the secret value"
        ),
    })?;
    if value.is_empty() {
        return Err(IronClawBuildError::InvalidConfig {
            reason: format!("{env_name} must not be empty"),
        });
    }
    Ok(SecretString::from(value))
}

fn require_postgres_resource_governor_singleton_env() -> Result<bool, IronClawBuildError> {
    match env_var_with_legacy(POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV) {
        Ok(value) => match parse_bool_opt_in(&value) {
            Some(true) => Ok(true),
            Some(false) => Err(IronClawBuildError::InvalidConfig {
                reason: format!(
                    "{POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV} must be true when this process is the singleton or elected resource-governor authority for the shared Postgres database"
                ),
            }),
            None => Err(IronClawBuildError::InvalidConfig {
                reason: format!(
                    "{POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV} must be one of true, false, 1, 0, yes, no, on, or off"
                ),
            }),
        },
        Err(std::env::VarError::NotPresent) => Err(IronClawBuildError::InvalidConfig {
            reason: format!(
                "{POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV} must be set to true when this process is the singleton or elected resource-governor authority for the shared Postgres database"
            ),
        }),
        Err(std::env::VarError::NotUnicode(_)) => Err(IronClawBuildError::InvalidConfig {
            reason: format!("{POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV} must be valid UTF-8"),
        }),
    }
}

fn postgres_pool_tls_options_from_env() -> Result<PostgresPoolTlsOptions, IronClawBuildError> {
    let ssl_mode_override =
        match std::env::var(DATABASE_SSLMODE_ENV) {
            Ok(value) if value.trim().is_empty() => None,
            Ok(value) => Some(value.trim().parse::<IronClawPostgresSslMode>().map_err(
                |error| IronClawBuildError::InvalidConfig {
                    reason: format!("{DATABASE_SSLMODE_ENV}: {error}"),
                },
            )?),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(IronClawBuildError::InvalidConfig {
                    reason: format!("{DATABASE_SSLMODE_ENV} must be valid UTF-8"),
                });
            }
        };
    let allow_remote_cleartext = match env_var_with_legacy(ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV) {
        Ok(value) => parse_bool_opt_in(&value).ok_or_else(|| {
            IronClawBuildError::InvalidConfig {
                reason: format!(
                    "{ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV} must be one of true, false, 1, 0, yes, no, on, or off"
                ),
            }
        })?,
        Err(std::env::VarError::NotPresent) => false,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(IronClawBuildError::InvalidConfig {
                reason: format!("{ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV} must be valid UTF-8"),
            });
        }
    };

    Ok(PostgresPoolTlsOptions {
        ssl_mode_override,
        allow_remote_cleartext,
    })
}

fn env_var_with_legacy(name: &str) -> Result<String, std::env::VarError> {
    env_var_with_legacy_from(name, |candidate| std::env::var(candidate))
}

fn env_var_with_legacy_from(
    name: &str,
    mut lookup: impl FnMut(&str) -> Result<String, std::env::VarError>,
) -> Result<String, std::env::VarError> {
    match lookup(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => {
            let Some(suffix) = name.strip_prefix("IRONCLAW_") else {
                return Err(std::env::VarError::NotPresent);
            };
            if suffix.starts_with("REBORN_") {
                return Err(std::env::VarError::NotPresent);
            }
            lookup(&format!("IRONCLAW_REBORN_{suffix}"))
        }
        Err(error) => Err(error),
    }
}

fn parse_bool_opt_in(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Some(false),
        "1" | "true" | "yes" | "on" => Some(true),
        _ => None,
    }
}

/// The DCR callback origin must be a bare https (or loopback http) origin.
fn validate_dcr_callback_origin(origin: &str) -> Result<(), AuthProductError> {
    let parsed = url::Url::parse(origin).map_err(|_| AuthProductError::BackendUnavailable)?;
    let is_loopback_http = parsed.scheme() == "http"
        && parsed
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]"));
    if parsed.scheme() != "https" && !is_loopback_http {
        return Err(AuthProductError::BackendUnavailable);
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AuthProductError::BackendUnavailable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_auth::InMemoryAuthProductServices;

    use super::*;

    #[test]
    fn with_product_auth_ports_records_injected_ports() {
        let product_auth = IronClawProductAuthServicePorts::from_shared(Arc::new(
            InMemoryAuthProductServices::new(),
        ));

        let input = IronClawBuildInput::disabled("test-owner")
            .with_product_auth_ports(product_auth.clone());

        assert!(input.product_auth_ports.is_some());
    }

    #[test]
    fn default_environment_name_wins_and_legacy_name_remains_a_fallback() {
        const DEFAULT: &str = "IRONCLAW_TEST_DEFAULT_CONTRACT";
        const LEGACY: &str = "IRONCLAW_REBORN_TEST_DEFAULT_CONTRACT";

        let legacy_only = |name: &str| match name {
            LEGACY => Ok("legacy".to_string()),
            _ => Err(std::env::VarError::NotPresent),
        };
        assert_eq!(
            env_var_with_legacy_from(DEFAULT, legacy_only).as_deref(),
            Ok("legacy"),
            "legacy names must remain accepted"
        );

        let both = |name: &str| match name {
            DEFAULT => Ok("default".to_string()),
            LEGACY => Ok("legacy".to_string()),
            _ => Err(std::env::VarError::NotPresent),
        };
        assert_eq!(
            env_var_with_legacy_from(DEFAULT, both).as_deref(),
            Ok("default"),
            "the neutral name must win when both are present"
        );
    }
}
