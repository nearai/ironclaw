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

use ironclaw_reborn_config::StorageBackend;
use ironclaw_reborn_event_store::{PostgresPoolTlsOptions, RebornPostgresSslMode};

use crate::RebornBuildError;
use crate::deployment::DeploymentConfig;
use crate::{RebornCompositionProfile, RebornProductAuthServicePorts};

const DEFAULT_REBORN_POSTGRES_URL_ENV: &str = "IRONCLAW_REBORN_POSTGRES_URL";
const DEFAULT_REBORN_SECRET_MASTER_KEY_ENV: &str = "IRONCLAW_REBORN_SECRET_MASTER_KEY";
const REBORN_POSTGRES_POOL_MAX_SIZE_ENV: &str = "IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE";
const REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON_ENV: &str =
    "IRONCLAW_REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON";
const DATABASE_SSLMODE_ENV: &str = "DATABASE_SSLMODE";
const ALLOW_REMOTE_POSTGRES_CLEAR_TEXT_ENV: &str =
    "IRONCLAW_REBORN_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT";

/// Composition-time OAuth client metadata.
///
/// `RebornHostBindings` owns this seam for product/bootstrap-provided values
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
pub enum RebornRuntimeProcessBinding {
    #[default]
    None,
    TenantSandbox {
        process_port: Arc<TenantSandboxProcessPort>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RebornRuntimeProcessBindingError {
    MissingTenantSandboxProcessPort,
    UnexpectedTenantSandboxProcessPort { process_backend: ProcessBackendKind },
}

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

pub struct RebornHostBindings {
    /// The deployment this build assembles, as data (§4.4/§5.6). Carries the
    /// substrate, traffic, readiness, and storage-shape axes every consumer
    /// reads instead of re-deriving them from a profile name.
    ///
    /// The **resolved** runtime policy rides `runtime_policy`, not this value:
    /// `new` builds the config without a yolo host-access disclosure (it is not
    /// known at construction), so callers that hold the operator's confirmation
    /// install the accurate config through
    /// [`RebornHostBindings::with_deployment`] — `local_runtime_build_input_with_options`
    /// is the one that does.
    pub(crate) deployment: DeploymentConfig,
    pub(crate) storage: RebornStorageInput,
    pub(crate) production_trust_policy: Option<Arc<HostTrustPolicy>>,
    pub(crate) turn_run_wake_notifier: Option<Arc<dyn TurnRunWakeNotifier>>,
    pub(crate) runtime_process_binding: RebornRuntimeProcessBinding,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) network_http_egress_for_test: Option<Arc<dyn NetworkHttpEgress>>,
    /// Test-support only: stamp filesystem-discovered extension packages as
    /// `HostBundled` so integration fixtures that model host-bundled
    /// extensions (the §8 invented-vendor fixture) may assert
    /// first-party trust. Production discovery always stamps
    /// `InstalledLocal` (#5459).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) trust_fixture_extensions_for_test: bool,
    pub(crate) product_auth_ports: Option<RebornProductAuthServicePorts>,
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
    /// Binary-assembled first-party capability handler registrars (GSuite,
    /// web tooling): composition runs each once against the shared registry so
    /// the concrete executors live in the binary, not composition.
    pub(crate) first_party_registrars:
        Vec<Arc<dyn crate::extension_host::first_party::FirstPartyHandlerRegistrar>>,
    /// Injected credential-account visibility policy (extension-family-aware,
    /// e.g. the GSuite account visibility policy). `None` falls back to the safe
    /// fail-closed default in the product-auth services.
    pub(crate) credential_account_visibility_policy: Option<
        Arc<
            dyn crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountVisibilityPolicy,
        >,
    >,
}

/// One channel extension's binary-assembled vendor binding
/// (extension-runtime DEL-7): the adapter linked into this deployment plus
/// the composition extras the generic channel host consumes.
/// Supplied through [`RebornHostBindings::with_channel_extension_bindings`] by
/// the assembling binary — composition itself never names a concrete
/// extension crate.
#[derive(Clone)]
pub struct ChannelExtensionBinding {
    /// The extension id the manifest declares (also the adapter id).
    pub extension_id: String,
    /// The channel adapter implementation linked into the deployment.
    pub adapter: std::sync::Arc<dyn ironclaw_product::ChannelAdapter>,
    /// The vendor half of the preference-target codec, consumed by the
    /// generic outbound-target provider and triggered-delivery hook.
    pub preference_target_codec:
        Option<std::sync::Arc<dyn ironclaw_product::PreferenceTargetCodec>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeOwnerIdentity {
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
}

/// Declarative PostgreSQL connection config (Phase B): the pure-data inputs
/// needed to open a pool at *build* time. Deliberately carries no live
/// `deadpool_postgres::Pool` handle — production resolves these values at
/// `RebornHostBindings` construction (reading env), but the pool is opened later
/// inside `build_production_shaped`.
#[derive(Clone)]
pub(crate) struct PostgresConnectionConfig {
    pub(crate) url: ironclaw_secrets::SecretMaterial,
    pub(crate) pool_max_size: usize,
    pub(crate) tls_options: PostgresPoolTlsOptions,
}

/// How the PostgreSQL pool is obtained at build time.
pub(crate) enum PostgresPoolSource {
    /// Production path: open the pool at build time from declarative config.
    Config(PostgresConnectionConfig),
    /// Test escape hatch: a caller-supplied, already-opened pool the build
    /// prefers over opening from config. Only the caller-supplied-handle
    /// constructors (`postgres`, `postgres_with_resolved_secret_master_key`,
    /// `hosted_single_tenant_postgres`) produce this; the
    /// `*_from_config_and_env` production constructors always use `Config`.
    Prebuilt(deadpool_postgres::Pool),
}

/// Declarative libSQL connection config (Phase B). `path_or_url` / `auth_token`
/// flow to the durable event-store config regardless of whether the database
/// handle is opened at build time or supplied pre-opened, so they live here
/// rather than inside [`RebornStorageInput::Libsql`]'s handle.
#[derive(Clone)]
pub(crate) struct LibsqlConnectionConfig {
    pub(crate) path_or_url: String,
    pub(crate) auth_token: Option<ironclaw_secrets::SecretMaterial>,
}

pub(crate) enum RebornStorageInput {
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
        pool_source: PostgresPoolSource,
        secret_master_key: ironclaw_secrets::SecretMaterial,
        process_local_resource_governor_singleton: bool,
    },
    Libsql {
        connection: LibsqlConnectionConfig,
        /// Test escape hatch: a caller-supplied, already-opened database the
        /// build prefers over opening from `connection`. When `None` the build
        /// opens the handle from `connection` at build time.
        prebuilt_db: Option<Arc<libsql::Database>>,
        secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
        process_local_resource_governor_singleton: bool,
    },
    Postgres {
        pool_source: PostgresPoolSource,
        secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
        process_local_resource_governor_singleton: bool,
    },
}

impl RebornHostBindings {
    /// Selected composition profile — a display/telemetry label. Behaviour
    /// comes from [`RebornHostBindings::deployment`].
    pub fn profile(&self) -> RebornCompositionProfile {
        self.deployment.profile()
    }

    /// The deployment axes this build assembles from.
    pub fn deployment(&self) -> &DeploymentConfig {
        &self.deployment
    }

    /// Install an accurately-resolved deployment config (Phase A). Used by
    /// [`RebornRuntimeInput::with_config`](crate::RebornRuntimeInput::with_config)
    /// to swap in a config built with the operator's yolo host-access disclosure
    /// after the bindings were constructed, preserving the declarative DATA the
    /// config now owns.
    pub fn with_deployment_config(mut self, deployment: DeploymentConfig) -> Self {
        self.deployment = deployment;
        self
    }

    /// Replace the deployment this input was constructed with.
    ///
    /// Test-only: production builds the deployment at construction
    /// (`RebornHostBindings::new` takes it, and `local_runtime_build_input_with_options`
    /// supplies one built where the operator's yolo disclosure is known). This
    /// exists so tests can construct a deliberately mismatched
    /// deployment/storage pairing and drive the fail-closed guard in
    /// `build_runtime_substrate` — production behaviour, reached through a
    /// pairing production rejects.
    #[cfg(test)]
    pub(crate) fn with_deployment(mut self, deployment: DeploymentConfig) -> Self {
        self.deployment = deployment;
        self
    }

    /// Owner id (string form). Used by the assembled runtime to mint the
    /// `UserId` actor for inbound CLI messages.
    pub fn owner_id(&self) -> &str {
        &self.deployment.owner_id
    }

    pub(crate) fn has_nearai_mcp_bootstrap_config(&self) -> bool {
        self.deployment.nearai_mcp_bootstrap_config.is_some()
    }

    /// Override the owner id after construction.
    ///
    /// The WebChat v2 serve path uses this to pin the runtime owner to the
    /// authenticated WebUI user *after* the runtime input (and its host-access
    /// disclosure gate) has been built, so the turn-runner loop host reads
    /// thread context from the same `owners/<user>` subtree the v2 facade
    /// wrote to.
    pub fn with_owner_id(mut self, owner_id: impl Into<String>) -> Self {
        self.deployment.owner_id = owner_id.into();
        self
    }

    /// Override the local runtime tenant/agent identity used by command-style
    /// facades that need a surface context before a full runtime exists.
    pub fn with_local_runtime_identity(mut self, tenant_id: TenantId, agent_id: AgentId) -> Self {
        self.deployment.local_runtime_identity = Some(RuntimeOwnerIdentity {
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

    /// Build a local-dev-storage-shaped input from an already-resolved
    /// deployment. The `debug_assert` is on the storage-shape **axis**, not on
    /// a list of profile names (§4.4).
    pub(crate) fn local_dev_from_deployment(
        deployment: DeploymentConfig,
        owner_id: impl Into<String>,
        root: PathBuf,
    ) -> Self {
        debug_assert!(deployment.uses_local_dev_storage_input());
        // Resolve the deployment's runtime policy from its policy request up
        // front, so a local-dev input is buildable without the caller
        // separately calling `.with_runtime_policy(...)`. This is what the
        // `local_runtime_build_input*` bridge did explicitly; folding it in here
        // removes the bare, unresolved-policy local-dev constructor that left
        // `runtime_policy` unset (and the build failing `MissingRuntimePolicy`).
        // Resolution is infallible for the non-yolo local-dev shapes; a yolo
        // shape without an acknowledged disclosure resolves to no policy, which
        // the caller can still override via `with_runtime_policy`.
        let resolved_policy = deployment.resolve().ok().flatten();
        let bindings = Self::new(
            deployment,
            owner_id,
            RebornStorageInput::LocalDev {
                root,
                workspace_root: None,
                host_home_root: None,
            },
        );
        match resolved_policy {
            Some(policy) => bindings.with_runtime_policy(policy),
            None => bindings,
        }
    }

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
                pool_source: PostgresPoolSource::Prebuilt(pool),
                secret_master_key,
                process_local_resource_governor_singleton: true,
            },
        ))
    }

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
            connection,
            secret_master_key,
            process_local_resource_governor_singleton,
        } = resolve_postgres_storage_from_config_and_env(profile, config_file)?;
        Ok(Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::HostedSingleTenantPostgres {
                root,
                workspace_root: None,
                host_home_root: None,
                pool_source: PostgresPoolSource::Config(connection),
                secret_master_key,
                process_local_resource_governor_singleton,
            },
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
        self.deployment
            .runtime_policy
            .as_ref()
            .is_some_and(|policy| {
                policy.filesystem_backend == FilesystemBackendKind::HostWorkspaceAndHome
            })
    }

    pub fn requires_local_dev_confirmed_host_home_root(&self) -> bool {
        self.requires_local_runtime_confirmed_host_home_root()
    }

    pub fn grants_trusted_laptop_access(&self) -> bool {
        self.deployment
            .runtime_policy
            .as_ref()
            .is_some_and(|policy| {
                policy.filesystem_backend == FilesystemBackendKind::HostWorkspaceAndHome
                    || policy.network_mode == NetworkMode::Direct
                    || policy.secret_mode == SecretMode::InheritedEnv
            })
    }

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
                connection: LibsqlConnectionConfig {
                    path_or_url: path_or_url.into(),
                    auth_token,
                },
                prebuilt_db: Some(db),
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton: true,
            },
        )
    }

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
                connection: LibsqlConnectionConfig {
                    path_or_url: path_or_url.into(),
                    auth_token,
                },
                prebuilt_db: Some(db),
                secret_master_key: None,
                process_local_resource_governor_singleton: true,
            },
        )
    }

    pub fn postgres(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        pool: deadpool_postgres::Pool,
        // Retained for API stability with the caller-supplied-handle test
        // escape hatch. The prebuilt pool is used verbatim, so no URL is opened.
        _url: ironclaw_secrets::SecretMaterial,
        secret_master_key: ironclaw_secrets::SecretMaterial,
    ) -> Self {
        Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::Postgres {
                pool_source: PostgresPoolSource::Prebuilt(pool),
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton: true,
            },
        )
    }

    pub fn postgres_with_resolved_secret_master_key(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        pool: deadpool_postgres::Pool,
        // Retained for API stability with the caller-supplied-handle test
        // escape hatch. The prebuilt pool is used verbatim, so no URL is opened.
        _url: ironclaw_secrets::SecretMaterial,
    ) -> Self {
        Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::Postgres {
                pool_source: PostgresPoolSource::Prebuilt(pool),
                secret_master_key: None,
                process_local_resource_governor_singleton: true,
            },
        )
    }

    pub fn postgres_from_config_and_env(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
    ) -> Result<Self, RebornBuildError> {
        let ResolvedPostgresStorage {
            connection,
            secret_master_key,
            process_local_resource_governor_singleton,
        } = resolve_postgres_storage_from_config_and_env(profile, config_file)?;
        let runtime_policy = resolve_production_runtime_policy(profile, config_file)?;

        // The built-in first-party trust policy is composed at BUILD time from
        // the binary-injected `first_party_bundles` (extension-runtime DEL-7),
        // not here — construction predates bundle injection. Leaving
        // `production_trust_policy` unset lets `build_production_shaped` source
        // the per-package effect grants from the injected bundle set.
        Ok(Self::new(
            DeploymentConfig::for_profile(profile, false),
            owner_id,
            RebornStorageInput::Postgres {
                pool_source: PostgresPoolSource::Config(connection),
                secret_master_key: Some(secret_master_key),
                process_local_resource_governor_singleton,
            },
        )
        .with_runtime_policy(runtime_policy)
        .with_runtime_process_binding(RebornRuntimeProcessBinding::none()))
    }

    pub fn with_required_runtime_backends(
        mut self,
        backends: impl IntoIterator<Item = ironclaw_host_api::RuntimeKind>,
    ) -> Self {
        self.deployment.required_runtime_backends = backends.into_iter().collect();
        self
    }

    pub fn with_production_trust_policy(mut self, policy: Arc<HostTrustPolicy>) -> Self {
        self.production_trust_policy = Some(policy);
        self
    }

    pub fn with_runtime_policy(mut self, policy: EffectiveRuntimePolicy) -> Self {
        self.deployment.runtime_policy = Some(policy);
        self
    }

    pub fn runtime_policy(&self) -> Option<&EffectiveRuntimePolicy> {
        self.deployment.runtime_policy.as_ref()
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
        self.deployment.require_runtime_http_egress = true;
        self
    }

    pub fn require_wasm_credentials(mut self) -> Self {
        self.deployment.require_wasm_credentials = true;
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
        descriptors: Vec<ironclaw_product::ExtensionAccountSetupDescriptor>,
    ) -> Self {
        self.deployment.account_setup_descriptors = descriptors;
        self
    }

    pub fn with_nearai_mcp_bootstrap_config(
        mut self,
        config: crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig,
    ) -> Self {
        self.deployment.nearai_mcp_bootstrap_config = Some(config);
        self
    }

    pub fn with_optional_nearai_mcp_bootstrap_config(
        mut self,
        config: Option<crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig>,
    ) -> Self {
        self.deployment.nearai_mcp_bootstrap_config = config;
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

    /// Trust filesystem-discovered fixture extensions as host-bundled
    /// (first-party-eligible). Test-support only; see the field doc.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_trusted_fixture_extensions_for_test(mut self) -> Self {
        self.trust_fixture_extensions_for_test = true;
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

    /// Record deployment OAuth client material for one vendor id. The vendor's
    /// manifest recipe names the client-credential handles these values fill.
    ///
    /// `RebornHostBindings` owns this composition seam until a settings-backed
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
        self.deployment.oauth_dcr_callback = Some(OAuthDcrCallbackConfig { callback_origin });
        Ok(self)
    }

    /// Set concurrency limits for the in-memory turn-state store.
    ///
    /// Called by `build_reborn_runtime` after mapping from `TurnRunnerSettings` so the
    /// factory can apply them when constructing the store. Callers should use
    /// `RebornRuntimeInput::with_runner_settings` rather than calling this directly.
    pub(crate) fn with_turn_state_store_limits(mut self, limits: TurnStateStoreLimits) -> Self {
        self.deployment.turn_state_store_limits = limits;
        self
    }

    fn push_oauth_provider_config(&mut self, vendor: String, client: OAuthClientConfig) {
        if let Some(existing) = self
            .deployment
            .oauth_provider_configs
            .iter_mut()
            .find(|existing| existing.vendor == vendor)
        {
            existing.client = client;
            return;
        }
        self.deployment
            .oauth_provider_configs
            .push(OAuthProviderBackendConfig { vendor, client });
    }

    fn new(
        deployment: DeploymentConfig,
        owner_id: impl Into<String>,
        storage: RebornStorageInput,
    ) -> Self {
        // Owner id is declarative DATA (Phase A) — carry it on the deployment,
        // not the bindings. Every other DATA field defaults on the deployment
        // preset and is overridden through the delegating builders below.
        let mut deployment = deployment;
        deployment.owner_id = owner_id.into();
        Self {
            deployment,
            storage,
            production_trust_policy: None,
            turn_run_wake_notifier: None,
            runtime_process_binding: RebornRuntimeProcessBinding::default(),
            #[cfg(any(test, feature = "test-support"))]
            network_http_egress_for_test: None,
            #[cfg(any(test, feature = "test-support"))]
            trust_fixture_extensions_for_test: false,
            product_auth_ports: None,
            native_extension_factories: Vec::new(),
            channel_extension_bindings: Vec::new(),
            first_party_registrars: Vec::new(),
            credential_account_visibility_policy: None,
        }
    }

    /// Inject the binary-assembled neutral first-party package inventory.
    pub fn with_first_party_bundles(
        mut self,
        bundles: Vec<crate::extension_host::first_party::FirstPartyPackageBundle>,
    ) -> Self {
        self.deployment.first_party_bundles = bundles;
        self
    }

    /// Inject the binary-assembled first-party capability handler registrars.
    pub fn with_first_party_registrars(
        mut self,
        registrars: Vec<Arc<dyn crate::extension_host::first_party::FirstPartyHandlerRegistrar>>,
    ) -> Self {
        self.first_party_registrars = registrars;
        self
    }

    /// Inject the credential-account visibility policy (see the field doc).
    pub fn with_credential_account_visibility_policy(
        mut self,
        policy: Arc<
            dyn crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountVisibilityPolicy,
        >,
    ) -> Self {
        self.credential_account_visibility_policy = Some(policy);
        self
    }

    /// Test-support: inject the full first-party extension surface (catalog
    /// bundles, capability-handler registrars, and the provider-account
    /// visibility policy) exactly as the `ironclaw_reborn_cli` binary does in
    /// production.
    ///
    /// Composition names no concrete first-party extension in production
    /// (extension-runtime DEL-7); the binary supplies these on the build input.
    /// Composition's own unit tests need the same surface to install / activate /
    /// dispatch first-party extensions through the production seam, so this
    /// mirrors the binary's assembly from the dev-dependency inventory. Gated
    /// `test-support` because integration harnesses compile composition as a
    /// dependency, not under composition's own `cfg(test)`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_bundled_first_party_for_test(self) -> Self {
        self.with_first_party_bundles(
            crate::extension_host::first_party::first_party_bundles_from_inventory(),
        )
        .with_first_party_registrars(
            crate::extension_host::first_party::test_support::bundled_first_party_registrars(),
        )
        .with_credential_account_visibility_policy(
            crate::extension_host::first_party::test_support::bundled_credential_account_visibility_policy(),
        )
    }
}

struct ResolvedPostgresStorage {
    connection: PostgresConnectionConfig,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
}

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
    ironclaw_reborn_event_store::validate_postgres_pool_tls_options(&database_url, tls_options)?;

    // Phase B: resolve the declarative connection config only. The live pool is
    // opened later, at build time, inside `build_production_shaped` — construction
    // no longer performs I/O against the database.
    Ok(ResolvedPostgresStorage {
        connection: PostgresConnectionConfig {
            url: database_url,
            pool_max_size,
            tls_options,
        },
        secret_master_key,
        process_local_resource_governor_singleton,
    })
}

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
        let product_auth = RebornProductAuthServicePorts::from_shared(Arc::new(
            InMemoryAuthProductServices::new(),
        ));

        let input = RebornHostBindings::disabled("test-owner")
            .with_product_auth_ports(product_auth.clone());

        assert!(input.product_auth_ports.is_some());
    }
}
