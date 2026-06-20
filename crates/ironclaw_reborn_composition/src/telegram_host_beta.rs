//! Host-beta Telegram updates composition (extension-state projection).
//!
//! Mirrors `slack_host_beta`'s extension-projection model. The CLI supplies an
//! explicit [`TelegramHostBetaConfig`]; [`import_telegram_host_beta_config_as_extension_installation`]
//! persists the host-owned settings + secret bindings into the bundled Telegram
//! extension, and [`build_telegram_updates_host_ingress_mount_from_enabled_extensions`]
//! projects the `/webhooks/telegram/updates` route from enabled extension state
//! through the generic host-ingress registry. The mounted route's descriptor is
//! projected from the manifest, so there is no hand-maintained path that can
//! drift out of the manifest.
//!
//! Two distinct secrets flow through here and must never be conflated:
//! * the **webhook secret** authenticates inbound updates (the host verifies the
//!   `X-Telegram-Bot-Api-Secret-Token` header before the adapter parses).
//! * the **bot token** authorizes outbound Bot API calls; it is injected into the
//!   request URL by [`crate::telegram_egress`] and never reaches the adapter.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_conversations::InMemoryConversationServices;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
    ExtensionInstallation, ExtensionInstallationId,
};
use ironclaw_host_api::ingress::{
    HostIngressRouteDeclaration, HostIngressTarget, IngressCredentialHandle,
};
use ironclaw_host_api::{
    AgentId, ExtensionId, InvocationId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
};
use ironclaw_host_ingress_registry::list_enabled_host_ingress_entries;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeclaredEgressTarget,
    EgressCredentialHandle, ProductAdapter, ProductAdapterId, ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, DefaultProductWorkflow, InMemoryIdempotencyLedger,
    LifecyclePackageKind, LifecyclePackageRef, ProductConversationBindingService,
    ProductInstallationKey, ProductInstallationScope, StaticProductInstallationResolver,
};
use ironclaw_secrets::{SecretMaterial, SecretStore};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TELEGRAM_API_HOST, TelegramV2Adapter, TelegramV2AdapterConfig,
};
use ironclaw_wasm_product_adapters::{
    EgressPolicy, ImmediateAckWorkflowObserver, NativeProductAdapterRunner,
    NativeProductAdapterRunnerConfig, SharedSecretHeaderAuth, WebhookAuth,
};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::RebornRuntime;
use crate::host_ingress::{HostIngressError, public_ingress_route_mount};
use crate::telegram_delivery::{
    TelegramFinalReplyDeliveryObserver, TelegramFinalReplyDeliveryServices,
};
use crate::telegram_egress::{StaticTelegramEgressCredentialProvider, TelegramProtocolHttpEgress};
use crate::telegram_extension_settings::{
    FilesystemTelegramExtensionSettingsStore, TelegramExtensionInstallationSettings,
};
use crate::telegram_host_ingress::{
    ExtensionInstallationIngressCredentialBinding, ExtensionInstallationIngressCredentialResolver,
    TELEGRAM_UPDATES_HOST_INGRESS_ROUTE_ID, TELEGRAM_WEBHOOK_SECRET_HEADER,
    TelegramHostIngressInstallation, TelegramUpdatesIngressHandler,
    telegram_updates_host_ingress_registrations,
};
use crate::webui_serve::PublicRouteMount;

const TELEGRAM_V2_ADAPTER_ID: &str = "telegram_v2";
/// Egress credential handle resolved by the host to the bot token at send time.
const TELEGRAM_BOT_TOKEN_HANDLE: &str = "telegram_bot_token";
/// Ingress credential handle / secret-store handle for the webhook shared secret.
const TELEGRAM_WEBHOOK_SECRET_HANDLE: &str = "telegram_webhook_secret";
const TELEGRAM_WEBHOOK_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(2);
const TELEGRAM_MAX_IN_FLIGHT_WEBHOOKS: usize = 64;
const TELEGRAM_EXTENSION_ID: &str = "telegram";

/// Explicit Telegram host config supplied by the CLI/serve layer. The bot token
/// and webhook secret are resolved from environment variables by the serve layer
/// and passed here as `SecretString`; they never appear in config files.
#[derive(Clone)]
pub struct TelegramHostBetaConfig {
    pub tenant_id: TenantId,
    pub installation_id: AdapterInstallationId,
    pub user_id: UserId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    /// Subject user for shared/group conversations; defaults to `user_id`.
    pub shared_subject_user_id: Option<UserId>,
    /// Bot username without a leading `@` (for group mention triggers).
    pub bot_username: String,
    /// Stable bot user id (for reply-to-bot triggers).
    pub bot_user_id: i64,
    /// Recognized bot commands without a leading `/`.
    pub recognized_commands: Vec<String>,
    pub bot_token: SecretString,
    pub webhook_secret: SecretString,
    pub progress_push_enabled: bool,
}

impl std::fmt::Debug for TelegramHostBetaConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramHostBetaConfig")
            .field("tenant_id", &self.tenant_id)
            .field("installation_id", &self.installation_id)
            .field("user_id", &self.user_id)
            .field("agent_id", &self.agent_id)
            .field("project_id", &self.project_id)
            .field("shared_subject_user_id", &self.shared_subject_user_id)
            .field("bot_username", &self.bot_username)
            .field("bot_user_id", &self.bot_user_id)
            .field("recognized_commands", &self.recognized_commands)
            .field("bot_token", &"<redacted>")
            .field("webhook_secret", &"<redacted>")
            .field("progress_push_enabled", &self.progress_push_enabled)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum TelegramHostBetaBuildError {
    #[error("Telegram host-beta requires durable host state")]
    DurableHostStateUnavailable,
    #[error("Telegram host-beta requires local runtime HTTP egress")]
    RuntimeHttpEgressUnavailable,
    #[error("Telegram host-beta requires extension lifecycle state")]
    ExtensionLifecycleUnavailable,
    #[error("Telegram host-beta requires the shared host secret store")]
    SecretStoreUnavailable,
    #[error("Telegram host-beta extension installation state failed: {reason}")]
    ExtensionInstallation { reason: String },
    #[error("Telegram host-beta extension settings failed: {reason}")]
    ExtensionSettings { reason: String },
    #[error("Telegram host-beta host ingress projection failed: {source}")]
    HostIngressProjection {
        #[from]
        source: ironclaw_host_ingress_registry::Error,
    },
    #[error("Telegram host-beta host ingress mount failed: {source}")]
    HostIngress {
        #[from]
        source: HostIngressError,
    },
    #[error("Telegram host-beta secret store failed: {source}")]
    HostIngressSecretStore {
        #[from]
        source: ironclaw_secrets::SecretStoreError,
    },
    #[error("invalid Telegram host-beta config field {field}: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
}

impl From<crate::telegram_extension_settings::Error> for TelegramHostBetaBuildError {
    fn from(error: crate::telegram_extension_settings::Error) -> Self {
        Self::ExtensionSettings {
            reason: error.to_string(),
        }
    }
}

fn invalid_config(field: &'static str, reason: impl Into<String>) -> TelegramHostBetaBuildError {
    TelegramHostBetaBuildError::InvalidConfig {
        field,
        reason: reason.into(),
    }
}

/// Persist an explicit Telegram host config into the bundled Telegram extension:
/// install the extension if needed, write the host-owned settings, store the bot
/// token + webhook secret in the secret store, and bind both to the enabled
/// installation. The mount is then projected from this state.
pub async fn import_telegram_host_beta_config_as_extension_installation(
    runtime: &RebornRuntime,
    config: &TelegramHostBetaConfig,
) -> Result<(), TelegramHostBetaBuildError> {
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(TelegramHostBetaBuildError::DurableHostStateUnavailable)?;
    let extension_management = local_runtime
        .extension_management
        .as_ref()
        .ok_or(TelegramHostBetaBuildError::ExtensionLifecycleUnavailable)?;
    let secret_store = local_runtime
        .secret_store
        .clone()
        .ok_or(TelegramHostBetaBuildError::SecretStoreUnavailable)?;
    let store = extension_management.installation_store();
    let extension_id = telegram_extension_id()?;
    let installation_id = telegram_extension_installation_id()?;
    if store
        .get_installation(&installation_id)
        .await
        .map_err(map_extension_installation_error)?
        .is_none()
    {
        extension_management
            .install(telegram_lifecycle_package_ref()?)
            .await
            .map_err(|error| TelegramHostBetaBuildError::ExtensionInstallation {
                reason: error.to_string(),
            })?;
    }

    let settings = TelegramExtensionInstallationSettings::from_host_beta_config(config)?;
    let settings_store = FilesystemTelegramExtensionSettingsStore::new(Arc::clone(
        &local_runtime.host_state_filesystem,
    ));
    settings_store.upsert(&installation_id, &settings).await?;

    let secret_scope = settings.secret_scope();
    let webhook_secret_handle = SecretHandle::new(TELEGRAM_WEBHOOK_SECRET_HANDLE)
        .map_err(|reason| invalid_config("telegram_webhook_secret_handle", reason.to_string()))?;
    let bot_token_handle = SecretHandle::new(TELEGRAM_BOT_TOKEN_HANDLE)
        .map_err(|reason| invalid_config("telegram_bot_token_handle", reason.to_string()))?;
    secret_store
        .put(
            secret_scope.clone(),
            webhook_secret_handle.clone(),
            SecretMaterial::from(config.webhook_secret.expose_secret().to_string()),
        )
        .await?;
    secret_store
        .put(
            secret_scope,
            bot_token_handle.clone(),
            SecretMaterial::from(config.bot_token.expose_secret().to_string()),
        )
        .await?;

    let current = store
        .get_installation(&installation_id)
        .await
        .map_err(map_extension_installation_error)?
        .ok_or_else(|| TelegramHostBetaBuildError::ExtensionInstallation {
            reason: "Telegram extension installation was not available after import install"
                .to_string(),
        })?;
    let imported = ExtensionInstallation::new(
        current.installation_id().clone(),
        extension_id,
        ExtensionActivationState::Enabled,
        current.manifest_ref().clone(),
        vec![
            ExtensionCredentialBinding::new(
                ExtensionCredentialHandle::new(TELEGRAM_WEBHOOK_SECRET_HANDLE)
                    .map_err(map_extension_installation_error)?,
                webhook_secret_handle,
            ),
            ExtensionCredentialBinding::new(
                ExtensionCredentialHandle::new(TELEGRAM_BOT_TOKEN_HANDLE)
                    .map_err(map_extension_installation_error)?,
                bot_token_handle,
            ),
        ],
        chrono::Utc::now(),
    )
    .map_err(map_extension_installation_error)?;
    store
        .upsert_installation(imported)
        .await
        .map_err(map_extension_installation_error)?;
    Ok(())
}

/// Project the Telegram updates webhook route from enabled extension state.
/// Returns `None` when no enabled Telegram extension declares the updates route.
pub async fn build_telegram_updates_host_ingress_mount_from_enabled_extensions(
    runtime: &RebornRuntime,
) -> Result<Option<PublicRouteMount>, TelegramHostBetaBuildError> {
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(TelegramHostBetaBuildError::DurableHostStateUnavailable)?;
    let extension_management = local_runtime
        .extension_management
        .as_ref()
        .ok_or(TelegramHostBetaBuildError::ExtensionLifecycleUnavailable)?;
    let secret_store = local_runtime
        .secret_store
        .clone()
        .ok_or(TelegramHostBetaBuildError::SecretStoreUnavailable)?;
    let store = extension_management.installation_store();
    let settings_store = FilesystemTelegramExtensionSettingsStore::new(Arc::clone(
        &local_runtime.host_state_filesystem,
    ));
    let entries = list_enabled_host_ingress_entries(store.as_ref()).await?;
    let telegram_extension_id = telegram_extension_id()?;
    let mut installations = Vec::new();
    let mut credential_bindings = Vec::new();

    for entry in entries {
        let installation = entry.installation();
        if installation.extension_id() != &telegram_extension_id {
            continue;
        }
        if !is_telegram_updates_declaration(entry.declaration()) {
            continue;
        }
        let settings = settings_store
            .get(installation.installation_id())
            .await?
            .ok_or_else(|| TelegramHostBetaBuildError::ExtensionInstallation {
                reason: format!(
                    "enabled Telegram extension installation `{}` is missing host-owned settings; rerun Telegram extension setup/import",
                    installation.installation_id()
                ),
            })?;
        let secret_scope = settings.secret_scope();
        let bot_secret_handle =
            extension_secret_handle(installation, TELEGRAM_BOT_TOKEN_HANDLE)?.clone();
        let webhook_secret_handle =
            extension_secret_handle(installation, TELEGRAM_WEBHOOK_SECRET_HANDLE)?.clone();
        let bot_token =
            read_secret_string(Arc::clone(&secret_store), &secret_scope, &bot_secret_handle)
                .await?;
        let webhook_secret = read_secret_string(
            Arc::clone(&secret_store),
            &secret_scope,
            &webhook_secret_handle,
        )
        .await?;
        let config = telegram_config_from_extension_settings(&settings, bot_token, webhook_secret)?;

        let bot_token_handle = telegram_bot_token_handle()?;
        let egress = telegram_protocol_egress(runtime, &config, bot_token_handle.clone())?;
        let (runner, observer) =
            build_telegram_runner_and_observer(runtime, &config, bot_token_handle, egress)?;

        let credential_handles = declaration_credential_handles(entry.declaration());
        for ingress_credential_handle in &credential_handles {
            let secret_handle =
                extension_secret_handle(installation, ingress_credential_handle.as_str())?;
            credential_bindings.push(ExtensionInstallationIngressCredentialBinding {
                candidate_id: config.installation_id.as_str().to_string(),
                ingress_credential_handle: ingress_credential_handle.clone(),
                secret_scope: secret_scope.clone(),
                secret_handle: secret_handle.clone(),
            });
        }
        installations.push(
            TelegramHostIngressInstallation::new(
                config.installation_id.clone(),
                credential_handles,
                runner,
            )?
            .with_workflow_observer(observer),
        );
    }

    if installations.is_empty() {
        return Ok(None);
    }

    let handler = Arc::new(TelegramUpdatesIngressHandler::new(installations)?);
    let resolver = Arc::new(ExtensionInstallationIngressCredentialResolver::new(
        secret_store,
        credential_bindings,
    )?);
    Ok(Some(public_ingress_route_mount(
        telegram_updates_host_ingress_registrations(handler)?,
        resolver,
    )?))
}

fn is_telegram_updates_declaration(declaration: &HostIngressRouteDeclaration) -> bool {
    if declaration.route().route_id().as_str() != TELEGRAM_UPDATES_HOST_INGRESS_ROUTE_ID {
        return false;
    }
    matches!(
        declaration.target(),
        HostIngressTarget::ProductAdapterInbound {
            product_adapter_section,
            ..
        } if product_adapter_section == "product_adapter.inbound"
    )
}

fn declaration_credential_handles(
    declaration: &HostIngressRouteDeclaration,
) -> Vec<IngressCredentialHandle> {
    declaration
        .auth()
        .iter()
        .flat_map(|binding| binding.credential_handles().iter().cloned())
        .collect()
}

fn extension_secret_handle<'a>(
    installation: &'a ExtensionInstallation,
    credential_handle: &str,
) -> Result<&'a SecretHandle, TelegramHostBetaBuildError> {
    installation
        .credential_bindings()
        .iter()
        .find(|binding| binding.credential_handle().as_str() == credential_handle)
        .map(|binding| binding.secret_handle())
        .ok_or_else(|| TelegramHostBetaBuildError::ExtensionInstallation {
            reason: format!(
                "enabled Telegram extension installation `{}` is missing credential binding `{credential_handle}`",
                installation.installation_id()
            ),
        })
}

async fn read_secret_string(
    secret_store: Arc<dyn SecretStore>,
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> Result<SecretString, TelegramHostBetaBuildError> {
    let lease = secret_store.lease_once(scope, handle).await?;
    Ok(secret_store.consume(scope, lease.id).await?)
}

fn telegram_config_from_extension_settings(
    settings: &TelegramExtensionInstallationSettings,
    bot_token: SecretString,
    webhook_secret: SecretString,
) -> Result<TelegramHostBetaConfig, TelegramHostBetaBuildError> {
    Ok(TelegramHostBetaConfig {
        tenant_id: settings.tenant_id.clone(),
        installation_id: settings.adapter_installation_id.clone(),
        user_id: settings.user_id.clone(),
        agent_id: settings.agent_id.clone(),
        project_id: settings.project_id.clone(),
        shared_subject_user_id: settings.shared_subject_user_id.clone(),
        bot_username: settings.bot_username.clone(),
        bot_user_id: settings.bot_user_id,
        recognized_commands: settings.recognized_commands.clone(),
        bot_token,
        webhook_secret,
        progress_push_enabled: settings.progress_push_enabled,
    })
}

fn telegram_extension_id() -> Result<ExtensionId, TelegramHostBetaBuildError> {
    ExtensionId::new(TELEGRAM_EXTENSION_ID)
        .map_err(|reason| invalid_config("extension_id", reason.to_string()))
}

fn telegram_extension_installation_id()
-> Result<ExtensionInstallationId, TelegramHostBetaBuildError> {
    ExtensionInstallationId::new(TELEGRAM_EXTENSION_ID).map_err(map_extension_installation_error)
}

fn telegram_lifecycle_package_ref() -> Result<LifecyclePackageRef, TelegramHostBetaBuildError> {
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, TELEGRAM_EXTENSION_ID).map_err(
        |error| TelegramHostBetaBuildError::ExtensionInstallation {
            reason: error.to_string(),
        },
    )
}

fn map_extension_installation_error(error: impl std::fmt::Display) -> TelegramHostBetaBuildError {
    TelegramHostBetaBuildError::ExtensionInstallation {
        reason: error.to_string(),
    }
}

type TelegramRunnerAndObserver = (
    Arc<NativeProductAdapterRunner>,
    Arc<dyn ImmediateAckWorkflowObserver>,
);

fn build_telegram_runner_and_observer(
    runtime: &RebornRuntime,
    config: &TelegramHostBetaConfig,
    bot_token_handle: EgressCredentialHandle,
    egress: Arc<dyn ProtocolHttpEgress>,
) -> Result<TelegramRunnerAndObserver, TelegramHostBetaBuildError> {
    let adapter_id = ProductAdapterId::new(TELEGRAM_V2_ADAPTER_ID)
        .map_err(|reason| invalid_config("adapter_id", reason.to_string()))?;
    let adapter: Arc<dyn ProductAdapter> =
        Arc::new(TelegramV2Adapter::new(TelegramV2AdapterConfig {
            adapter_id: adapter_id.clone(),
            installation_id: config.installation_id.clone(),
            group_trigger_policy: GroupTriggerPolicy {
                bot_username: config.bot_username.clone(),
                bot_user_id: config.bot_user_id,
                recognized_commands: config.recognized_commands.clone(),
            },
            egress_credential_handle: bot_token_handle,
            auth_requirement: AuthRequirement::SharedSecretHeader {
                header_name: TELEGRAM_WEBHOOK_SECRET_HEADER.to_string(),
            },
            progress_push_enabled: config.progress_push_enabled,
        }));

    let conversations = Arc::new(InMemoryConversationServices::default());
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let scope = ProductInstallationScope::with_default_scope(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    )
    .with_default_subject_user_id(
        config
            .shared_subject_user_id
            .clone()
            .unwrap_or_else(|| config.user_id.clone()),
    );
    let installation_resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(adapter_id, config.installation_id.clone()),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, installation_resolver);

    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    ));
    // In-memory idempotency ledger deduplicates Telegram `update_id` retries
    // within the process. `DefaultProductWorkflow` defaults the delivered-gate
    // store and approval/auth interaction services, so the Telegram path needs
    // none of the Slack-specific durable host wiring.
    let workflow = Arc::new(DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::default()),
        Arc::new(binding.clone()),
    ));

    // Final-reply delivery observer: after the immediate ACK, it awaits run
    // completion and renders the assistant reply back to the chat through the
    // same adapter + host-mediated egress.
    let observer: Arc<dyn ImmediateAckWorkflowObserver> = Arc::new(
        TelegramFinalReplyDeliveryObserver::new(TelegramFinalReplyDeliveryServices {
            binding_service: Arc::new(binding.clone()),
            thread_service: runtime.webui_thread_service(),
            turn_coordinator: runtime.webui_turn_coordinator(),
            adapter: Arc::clone(&adapter),
            egress,
            delivery_sink: Arc::new(NoopTelegramDeliverySink),
        }),
    );

    // The runner's webhook auth is the real webhook secret: the host-ingress
    // path pre-verifies the shared-secret header and dispatches via the verified
    // immediate-ACK entrypoint, but `SharedSecretHeaderAuth` fails closed, so the
    // runner must never carry a sentinel that real requests cannot match.
    let runner = Arc::new(NativeProductAdapterRunner::with_config(
        adapter,
        workflow,
        WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
            header_name: TELEGRAM_WEBHOOK_SECRET_HEADER.to_string(),
            expected_secret: config.webhook_secret.expose_secret().to_string(),
            subject: config.installation_id.as_str().to_string(),
        }),
        NativeProductAdapterRunnerConfig::new(
            TELEGRAM_WEBHOOK_WORKFLOW_TIMEOUT,
            NonZeroUsize::new(TELEGRAM_MAX_IN_FLIGHT_WEBHOOKS)
                .ok_or_else(|| invalid_config("max_in_flight", "must be non-zero".to_string()))?,
        ),
    ));

    Ok((runner, observer))
}

/// No-op outbound delivery sink. The Telegram adapter records a `DeliveryStatus`
/// per send; surfacing those through the durable outbound store/WebUI is a
/// follow-up, so the first slice discards them while still delivering the reply.
struct NoopTelegramDeliverySink;

#[async_trait::async_trait]
impl ironclaw_product_adapters::OutboundDeliverySink for NoopTelegramDeliverySink {
    async fn record(&self, _status: ironclaw_product_adapters::DeliveryStatus) {}
}

/// Build the host-mediated Telegram Bot API egress (outbound). The bot token is
/// held only by the egress credential provider and injected into the request URL
/// by [`crate::telegram_egress`]; it never reaches the adapter.
pub fn telegram_protocol_egress(
    runtime: &RebornRuntime,
    config: &TelegramHostBetaConfig,
    bot_token_handle: EgressCredentialHandle,
) -> Result<Arc<dyn ProtocolHttpEgress>, TelegramHostBetaBuildError> {
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(TelegramHostBetaBuildError::RuntimeHttpEgressUnavailable)?;
    let host_egress = local_runtime
        .host_runtime_http_egress
        .clone()
        .ok_or(TelegramHostBetaBuildError::RuntimeHttpEgressUnavailable)?;
    let egress = TelegramProtocolHttpEgress::new(
        host_egress,
        Arc::new(StaticTelegramEgressCredentialProvider::new(
            bot_token_handle.clone(),
            config.bot_token.expose_secret().to_string(),
        )),
        EgressPolicy::new(telegram_declared_egress_targets(bot_token_handle)?),
        telegram_egress_scope_template(config),
    )
    .with_base_url_override_from_env()
    .map_err(|error| invalid_config("telegram_api_base_url", error.to_string()))?;
    Ok(Arc::new(egress))
}

fn telegram_declared_egress_targets(
    bot_token_handle: EgressCredentialHandle,
) -> Result<Vec<DeclaredEgressTarget>, TelegramHostBetaBuildError> {
    let host = DeclaredEgressHost::new(TELEGRAM_API_HOST)
        .map_err(|reason| invalid_config("telegram_api_host", reason.to_string()))?;
    Ok(vec![DeclaredEgressTarget::new(
        host,
        Some(bot_token_handle),
    )])
}

fn telegram_bot_token_handle() -> Result<EgressCredentialHandle, TelegramHostBetaBuildError> {
    EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_HANDLE)
        .map_err(|reason| invalid_config("telegram_bot_token_handle", reason.to_string()))
}

fn telegram_egress_scope_template(config: &TelegramHostBetaConfig) -> ResourceScope {
    ResourceScope {
        tenant_id: config.tenant_id.clone(),
        user_id: config.user_id.clone(),
        agent_id: Some(config.agent_id.clone()),
        project_id: config.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}
