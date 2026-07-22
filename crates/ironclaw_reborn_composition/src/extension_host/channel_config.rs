//! Generic `[channel.config]` configure surface (extension-runtime §6.4–§6.5).
//!
//! [`ChannelConfigService`] is the single production writer of operator
//! channel configuration: it validates submitted values against the
//! installed manifest's `[channel.config]` field descriptors, routes
//! non-secret values to the durable installation store (they ride
//! `InstallationRecord.config` into `ChannelAdapter::activate`), and routes
//! secret values into the shared scoped secret store under the
//! manifest-declared handle — the same scope the channel egress credential
//! fallback reads, so stored secrets resolve at egress time with no bridge.
//!
//! Editing config while the extension is Active runs the §6.5 automatic
//! deactivate → reactivate cycle through the generic host (adapters are
//! rebuilt with the new values and `activate()` revalidates them); the
//! cycle itself is owned by the lifecycle port behind
//! [`ChannelConfigReactivation`]. Stored secret values are never echoed
//! back: [`ChannelConfigService::status`] reports presence only.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionInstallationStore, ExtensionManifestRecord, ResolvedExtensionManifest,
};
use ironclaw_host_api::{ExtensionId, RecipeSecretField, ResourceScope, SecretHandle};
use ironclaw_product_workflow::{ProductWorkflowError, RebornServicesError};
use ironclaw_secrets::{SecretMaterial, SecretStore};

use crate::extension_host::admin_configuration::ComposedAdminConfigurationService;

/// Presence-only projection of one `[channel.config]` field (§6.4 config
/// completeness input). Secret fields never carry their stored value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelConfigFieldStatus {
    pub(crate) handle: String,
    pub(crate) label: String,
    pub(crate) secret: bool,
    pub(crate) provided: bool,
}

/// Typed configure-surface failures. Never carries secret material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ChannelConfigError {
    #[error("extension {extension_id} is not installed")]
    NotInstalled { extension_id: String },
    #[error("field `{handle}` is not declared by the extension's channel configuration")]
    UnknownField { handle: String },
    #[error("channel configuration storage failed: {reason}")]
    Storage { reason: String },
    #[error("channel reactivation failed: {reason}")]
    Reactivation { reason: String },
}

/// Runs the §6.5 deactivate → reactivate cycle for an extension whose
/// channel config changed, if (and only if) it is currently active. The
/// lifecycle port implements this over the generic host; a no-op when the
/// extension is not active or no host is attached.
#[async_trait]
pub(crate) trait ChannelConfigReactivation: Send + Sync {
    async fn reactivate_if_active(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError>;
}

/// The generic configure surface over the durable installation store and
/// the shared scoped secret store.
pub(crate) struct ChannelConfigService {
    installation_store: Arc<dyn ExtensionInstallationStore>,
    secrets: Arc<dyn SecretStore>,
    /// The channel-egress credential scope: secrets stored here resolve
    /// through the egress credential fallback with no bridging.
    secret_scope: ResourceScope,
    reactivation: Arc<dyn ChannelConfigReactivation>,
    admin_configuration: Option<AdminConfigurationConsumer>,
    available_manifests: BTreeMap<ExtensionId, Arc<ResolvedExtensionManifest>>,
}

#[derive(Clone)]
struct AdminConfigurationConsumer {
    service: Arc<ComposedAdminConfigurationService>,
    scope: ResourceScope,
}

impl ChannelConfigService {
    pub(crate) fn new(
        installation_store: Arc<dyn ExtensionInstallationStore>,
        secrets: Arc<dyn SecretStore>,
        secret_scope: ResourceScope,
        reactivation: Arc<dyn ChannelConfigReactivation>,
    ) -> Self {
        Self {
            installation_store,
            secrets,
            secret_scope,
            reactivation,
            admin_configuration: None,
            available_manifests: BTreeMap::new(),
        }
    }

    pub(crate) fn with_admin_configuration(
        mut self,
        service: Arc<ComposedAdminConfigurationService>,
        scope: ResourceScope,
    ) -> Self {
        self.admin_configuration = Some(AdminConfigurationConsumer { service, scope });
        self
    }

    /// Attach the read-only available-catalog projection used by
    /// deployment-owned channel ingress. This does not create an extension
    /// installation; it only lets generic configuration consumers resolve
    /// manifest-declared admin fields before a user installs the extension.
    pub(crate) fn with_available_manifests(
        mut self,
        manifests: impl IntoIterator<Item = Arc<ResolvedExtensionManifest>>,
    ) -> Self {
        self.available_manifests = manifests
            .into_iter()
            .map(|manifest| (manifest.id.clone(), manifest))
            .collect();
        self
    }

    async fn manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ExtensionManifestRecord, ChannelConfigError> {
        self.installation_store
            .get_manifest(extension_id)
            .await
            .map_err(storage_error)?
            .ok_or_else(|| ChannelConfigError::NotInstalled {
                extension_id: extension_id.as_str().to_string(),
            })
    }

    /// The installed manifest's `[channel.config]` field descriptors; empty
    /// for an extension without a channel surface (nothing to configure).
    async fn descriptors(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<RecipeSecretField>, ChannelConfigError> {
        let record = self.manifest(extension_id).await?;
        Ok(record
            .resolved()
            .channel
            .as_ref()
            .map(|channel| channel.config.fields.clone())
            .unwrap_or_default())
    }

    async fn resolved_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Arc<ResolvedExtensionManifest>, ChannelConfigError> {
        if let Some(record) = self
            .installation_store
            .get_manifest(extension_id)
            .await
            .map_err(storage_error)?
        {
            return Ok(Arc::new(record.resolved().clone()));
        }
        self.available_manifests
            .get(extension_id)
            .cloned()
            .ok_or_else(|| ChannelConfigError::NotInstalled {
                extension_id: extension_id.as_str().to_string(),
            })
    }

    /// Save submitted `(handle, value)` pairs. Handles must be declared by
    /// the installed manifest (unknown handle = typed error, nothing
    /// stored). Missing fields may stay absent — activation validates
    /// completeness. Blank secret submissions leave the stored material
    /// unchanged (parity with credential setup semantics). A save that
    /// changed anything triggers the §6.5 reactivate cycle when the
    /// extension is currently active.
    pub(crate) async fn save(
        &self,
        extension_id: &ExtensionId,
        values: Vec<(String, String)>,
    ) -> Result<(), ChannelConfigError> {
        if values.is_empty() {
            return Ok(());
        }
        let descriptors = self.descriptors(extension_id).await?;
        for (handle, _) in &values {
            if !descriptors
                .iter()
                .any(|field| field.handle.as_str() == handle)
            {
                return Err(ChannelConfigError::UnknownField {
                    handle: handle.clone(),
                });
            }
        }

        let mut non_secret = self
            .installation_store
            .channel_config(extension_id)
            .await
            .map_err(storage_error)?;
        let mut non_secret_changed = false;
        let mut secret_stored = false;
        for (handle, value) in values {
            let Some(descriptor) = descriptors
                .iter()
                .find(|field| field.handle.as_str() == handle)
            else {
                // Unreachable: validated above; kept fail-closed.
                return Err(ChannelConfigError::UnknownField { handle });
            };
            if descriptor.secret {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    continue;
                }
                self.secrets
                    .put(
                        self.secret_scope.clone(),
                        descriptor.handle.clone(),
                        SecretMaterial::from(trimmed.to_string()),
                        None,
                    )
                    .await
                    .map_err(storage_error)?;
                secret_stored = true;
            } else {
                match non_secret.iter_mut().find(|(stored, _)| *stored == handle) {
                    Some(entry) if entry.1 == value => {}
                    Some(entry) => {
                        entry.1 = value;
                        non_secret_changed = true;
                    }
                    None => {
                        non_secret.push((handle, value));
                        non_secret_changed = true;
                    }
                }
            }
        }
        if non_secret_changed {
            self.installation_store
                .set_channel_config(extension_id, non_secret)
                .await
                .map_err(storage_error)?;
        }
        if non_secret_changed || secret_stored {
            self.reactivation
                .reactivate_if_active(extension_id)
                .await
                .map_err(|error| ChannelConfigError::Reactivation {
                    reason: error.to_string(),
                })?;
        }
        Ok(())
    }

    /// Read one stored secret's material from the channel-config secret
    /// storage (the scoped secret store at the channel-egress credential
    /// scope, under the manifest-declared handle). `None` when the secret
    /// was never provided. Per-request read: a configure save takes effect
    /// on the next resolution with no rewiring — the ingress verification
    /// port resolves `verification.secret_handle` through this.
    pub(crate) async fn secret_material(
        &self,
        extension_id: &ExtensionId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, ChannelConfigError> {
        if let Some(admin) = &self.admin_configuration {
            let manifest = self.resolved_manifest(extension_id).await?;
            if let Some(descriptor) = manifest.admin_configuration.iter().find(|descriptor| {
                descriptor
                    .fields
                    .iter()
                    .any(|field| field.secret && field.handle == *handle)
            }) && let Some(material) = admin
                .service
                .secret_material(&admin.scope, &descriptor.group_id, handle)
                .await
                .map_err(admin_configuration_error)?
            {
                return Ok(Some(material));
            }
        }
        let lease = match self.secrets.lease_once(&self.secret_scope, handle).await {
            Ok(lease) => lease,
            Err(error) if error.is_unknown_secret() => return Ok(None),
            Err(error) => return Err(storage_error(error)),
        };
        self.secrets
            .consume(&self.secret_scope, lease.id)
            .await
            .map(Some)
            .map_err(storage_error)
    }

    /// Read one stored non-secret `[channel.config]` value from the durable
    /// installation config. Per-request read: a configure save takes effect
    /// on the next resolution with no rewiring — shared-channel admission
    /// resolves its routing values through this.
    pub(crate) async fn non_secret_value(
        &self,
        extension_id: &ExtensionId,
        handle: &str,
    ) -> Result<Option<String>, ChannelConfigError> {
        if let Some(admin) = &self.admin_configuration {
            let manifest = self.resolved_manifest(extension_id).await?;
            let handle = SecretHandle::new(handle).map_err(storage_error)?;
            if let Some(descriptor) = manifest.admin_configuration.iter().find(|descriptor| {
                descriptor
                    .fields
                    .iter()
                    .any(|field| !field.secret && field.handle == handle)
            }) && let Some(value) = admin
                .service
                .non_secret_value(&admin.scope, &descriptor.group_id, &handle)
                .await
                .map_err(admin_configuration_error)?
            {
                return Ok(Some(value));
            }
        }
        let values = self
            .installation_store
            .channel_config(extension_id)
            .await
            .map_err(storage_error)?;
        Ok(values
            .into_iter()
            .find(|(stored, _)| stored == handle)
            .map(|(_, value)| value))
    }

    /// Resolve one manifest-declared non-secret configuration handle across
    /// the active/available extension catalog. Secret fields are deliberately
    /// excluded because these values may be placed in a user-visible OAuth
    /// authorization URL.
    pub(crate) async fn non_secret_handle_value(
        &self,
        handle: &SecretHandle,
    ) -> Result<Option<String>, ChannelConfigError> {
        let installed_manifests = self
            .installation_store
            .list_manifests()
            .await
            .map_err(storage_error)?;

        if let Some(admin) = &self.admin_configuration {
            let available = self.available_manifests.values().cloned();
            let installed = installed_manifests
                .iter()
                .map(|record| Arc::new(record.resolved().clone()));
            for manifest in available.chain(installed) {
                let Some(descriptor) = manifest.admin_configuration.iter().find(|descriptor| {
                    descriptor
                        .fields
                        .iter()
                        .any(|field| !field.secret && field.handle == *handle)
                }) else {
                    continue;
                };
                if let Some(value) = admin
                    .service
                    .non_secret_value(&admin.scope, &descriptor.group_id, handle)
                    .await
                    .map_err(admin_configuration_error)?
                {
                    return Ok(Some(value));
                }
            }
        }

        for record in installed_manifests {
            let Some(channel) = record.resolved().channel.as_ref() else {
                continue;
            };
            if channel
                .config
                .fields
                .iter()
                .any(|field| !field.secret && field.handle == *handle)
            {
                return self
                    .non_secret_value(&record.resolved().id, handle.as_str())
                    .await;
            }
        }
        Ok(None)
    }

    /// Resolve one auth-recipe client-credential handle from manifest-declared
    /// administrator configuration, including extensions with no channel
    /// surface. The retired channel-config store remains a compatibility
    /// fallback for older manifests. Per-request resolution means an operator
    /// save takes effect on the next OAuth start without rewiring.
    pub(crate) async fn credential_handle_value(
        &self,
        handle: &str,
    ) -> Result<Option<secrecy::SecretString>, ChannelConfigError> {
        let installed_manifests = self
            .installation_store
            .list_manifests()
            .await
            .map_err(storage_error)?;
        let typed_handle = SecretHandle::new(handle).map_err(storage_error)?;
        if let Some(admin) = &self.admin_configuration {
            let available = self.available_manifests.values().cloned();
            let installed = installed_manifests
                .iter()
                .map(|record| Arc::new(record.resolved().clone()));
            for manifest in available.chain(installed) {
                let Some((descriptor, field)) =
                    manifest.admin_configuration.iter().find_map(|descriptor| {
                        descriptor
                            .fields
                            .iter()
                            .find(|field| field.handle == typed_handle)
                            .map(|field| (descriptor, field))
                    })
                else {
                    continue;
                };
                if field.secret {
                    let material = admin
                        .service
                        .secret_material(&admin.scope, &descriptor.group_id, &typed_handle)
                        .await
                        .map_err(admin_configuration_error)?;
                    if let Some(material) = material {
                        return Ok(Some(secrecy::SecretString::from(
                            secrecy::ExposeSecret::expose_secret(&material).to_string(),
                        )));
                    }
                } else if let Some(value) = admin
                    .service
                    .non_secret_value(&admin.scope, &descriptor.group_id, &typed_handle)
                    .await
                    .map_err(admin_configuration_error)?
                {
                    return Ok(Some(secrecy::SecretString::from(value)));
                }
            }
        }
        for record in installed_manifests {
            let Some(channel) = record.resolved().channel.as_ref() else {
                continue;
            };
            let Some(field) = channel
                .config
                .fields
                .iter()
                .find(|field| field.handle.as_str() == handle)
            else {
                continue;
            };
            if field.secret {
                let extension_id = record.resolved().id.clone();
                let material = self.secret_material(&extension_id, &field.handle).await?;
                return Ok(material.map(|material| {
                    secrecy::SecretString::from(
                        secrecy::ExposeSecret::expose_secret(&material).to_string(),
                    )
                }));
            }
            let extension_id = record.resolved().id.clone();
            return self
                .non_secret_value(&extension_id, handle)
                .await
                .map(|value| value.map(secrecy::SecretString::from));
        }
        Ok(None)
    }

    /// Resolve the non-secret configuration passed to `ChannelAdapter::activate`.
    /// Manifest-declared tenant admin values take precedence over the retired
    /// per-installation configure surface while preserving it as a compatibility
    /// fallback for manifests that do not declare administrator configuration.
    pub(crate) async fn effective_non_secret_config(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<(String, String)>, ChannelConfigError> {
        let manifest = self.resolved_manifest(extension_id).await?;
        let mut effective = self
            .installation_store
            .channel_config(extension_id)
            .await
            .map_err(storage_error)?;
        let Some(admin) = &self.admin_configuration else {
            return Ok(effective);
        };
        let Some(channel) = manifest.channel.as_ref() else {
            return Ok(effective);
        };
        for descriptor in &manifest.admin_configuration {
            for field in descriptor.fields.iter().filter(|field| {
                !field.secret
                    && channel
                        .config
                        .fields
                        .iter()
                        .any(|channel_field| channel_field.handle == field.handle)
            }) {
                let Some(value) = admin
                    .service
                    .non_secret_value(&admin.scope, &descriptor.group_id, &field.handle)
                    .await
                    .map_err(admin_configuration_error)?
                else {
                    continue;
                };
                match effective
                    .iter_mut()
                    .find(|(handle, _)| handle == field.handle.as_str())
                {
                    Some(existing) => existing.1 = value,
                    None => effective.push((field.handle.as_str().to_string(), value)),
                }
            }
        }
        Ok(effective)
    }

    /// Per-field presence for the extension's `[channel.config]` fields
    /// (§6.4 derived config completeness). Secret fields report
    /// `provided` only — stored values are never echoed back.
    pub(crate) async fn status(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<ChannelConfigFieldStatus>, ChannelConfigError> {
        let manifest = self.resolved_manifest(extension_id).await?;
        let descriptors = manifest
            .channel
            .as_ref()
            .map(|channel| channel.config.fields.clone())
            .unwrap_or_default();
        if descriptors.is_empty() {
            return Ok(Vec::new());
        }
        let stored = self
            .installation_store
            .channel_config(extension_id)
            .await
            .map_err(storage_error)?;
        let mut statuses = Vec::with_capacity(descriptors.len());
        for field in descriptors {
            let admin_provided = if let Some(admin) = &self.admin_configuration
                && let Some(descriptor) = manifest.admin_configuration.iter().find(|descriptor| {
                    descriptor
                        .fields
                        .iter()
                        .any(|admin_field| admin_field.handle == field.handle)
                }) {
                admin
                    .service
                    .get(&admin.scope, &descriptor.group_id)
                    .await
                    .map_err(admin_configuration_error)?
                    .fields
                    .into_iter()
                    .find(|admin_field| admin_field.handle == field.handle)
                    .is_some_and(|admin_field| admin_field.provided)
            } else {
                false
            };
            let legacy_provided = if field.secret {
                self.secrets
                    .metadata(&self.secret_scope, &field.handle)
                    .await
                    .map_err(storage_error)?
                    .is_some()
            } else {
                stored
                    .iter()
                    .any(|(handle, _)| handle == field.handle.as_str())
            };
            statuses.push(ChannelConfigFieldStatus {
                handle: field.handle.as_str().to_string(),
                label: field.label,
                secret: field.secret,
                provided: admin_provided || legacy_provided,
            });
        }
        Ok(statuses)
    }
}

fn storage_error(error: impl std::fmt::Display) -> ChannelConfigError {
    ChannelConfigError::Storage {
        reason: error.to_string(),
    }
}

fn admin_configuration_error(
    error: ironclaw_extension_host::AdminConfigurationServiceError,
) -> ChannelConfigError {
    tracing::warn!(error = %error, "admin configuration consumer resolution failed");
    ChannelConfigError::Storage {
        reason: "administrator configuration is unavailable".to_string(),
    }
}

/// The production [`ironclaw_product_workflow::ChannelConfigFacade`] port
/// over [`ChannelConfigService`] — the surface the WebUI setup facade and
/// the lifecycle configure action route through.
pub(crate) struct RebornChannelConfigFacade {
    service: Arc<ChannelConfigService>,
}

impl RebornChannelConfigFacade {
    pub(crate) fn new(service: Arc<ChannelConfigService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl ironclaw_product_workflow::ChannelConfigFacade for RebornChannelConfigFacade {
    async fn field_status(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<ironclaw_product_workflow::RebornChannelConfigField>, RebornServicesError> {
        match self.service.status(extension_id).await {
            Ok(statuses) => Ok(statuses
                .into_iter()
                .map(
                    |status| ironclaw_product_workflow::RebornChannelConfigField {
                        name: status.handle,
                        label: status.label,
                        secret: status.secret,
                        provided: status.provided,
                    },
                )
                .collect()),
            // A not-yet-installed extension has nothing to configure; the
            // setup view renders for it, so this projection stays empty
            // rather than erroring.
            Err(ChannelConfigError::NotInstalled { .. }) => Ok(Vec::new()),
            Err(error) => Err(map_channel_config_error(error)),
        }
    }

    async fn save_values(
        &self,
        extension_id: &ExtensionId,
        values: Vec<(String, String)>,
    ) -> Result<(), RebornServicesError> {
        self.service
            .save(extension_id, values)
            .await
            .map_err(map_channel_config_error)
    }
}

fn map_channel_config_error(error: ChannelConfigError) -> RebornServicesError {
    use ironclaw_product_workflow::{RebornServicesErrorCode, RebornServicesErrorKind};
    match error {
        ChannelConfigError::NotInstalled { .. } => RebornServicesError {
            code: RebornServicesErrorCode::NotFound,
            kind: RebornServicesErrorKind::NotFound,
            status_code: 404,
            retryable: false,
            field: None,
            validation_code: None,
        },
        ChannelConfigError::UnknownField { .. } => RebornServicesError {
            code: RebornServicesErrorCode::InvalidRequest,
            kind: RebornServicesErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: None,
            validation_code: None,
        },
        ChannelConfigError::Storage { .. } => RebornServicesError {
            code: RebornServicesErrorCode::Unavailable,
            kind: RebornServicesErrorKind::ServiceUnavailable,
            status_code: 503,
            retryable: true,
            field: None,
            validation_code: None,
        },
        // The save persisted but the §6.5 reactivate cycle failed: the host
        // record is left per §6.1 with the typed reason; the operator fixes
        // the value and saves again.
        ChannelConfigError::Reactivation { .. } => RebornServicesError {
            code: RebornServicesErrorCode::Conflict,
            kind: RebornServicesErrorKind::Conflict,
            status_code: 409,
            retryable: false,
            field: None,
            validation_code: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_extension_host::{
        AdminConfigurationIdempotencyKey, AdminConfigurationService,
        AdminConfigurationSubmittedValue, FilesystemAdminConfigurationStore,
    };
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
        ExtensionManifestRecord, ExtensionManifestRef, FilesystemExtensionInstallationStore,
        ManifestSource,
    };
    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{InvocationId, SecretHandle, UserId};
    use ironclaw_secrets::FilesystemSecretStore;

    use super::*;
    use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

    /// An invented pure-channel extension declaring two secret fields and
    /// one non-secret field — the fixture for every test here.
    const CHANNEL_FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acmechat"
name = "AcmeChat"
version = "0.1.0"
description = "channel-config service fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "acmechat.extension/v1"

[channel]
id = "messages"
display_name = "AcmeChat messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "acmechat_webhook_secret"
header = "X-AcmeChat-Secret"

[channel.config]
fields = [
  { handle = "acmechat_api_token", label = "API token", secret = true },
  { handle = "acmechat_webhook_secret", label = "Webhook secret", secret = true },
  { handle = "acmechat_public_url", label = "Public URL", secret = false },
]

[[channel.egress]]
scheme = "https"
host = "api.acmechat.example"
methods = ["post"]
credential_handle = "acmechat_api_token"
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[channel.presentation]
supports_markdown = false
supports_threads = false
"#;

    /// A tools-only fixture (no channel surface): nothing to configure.
    const TOOLS_ONLY_FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "zephyrite"
name = "Zephyrite"
version = "0.1.0"
description = "tools-only fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/zephyrite_tool.wasm"

[[tools]]
id = "zephyrite.echo"
description = "Echoes input"
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/zephyrite/echo.input.v1.json"
"#;

    const NON_CHANNEL_ADMIN_FIXTURE_MANIFEST: &str =
        include_str!("../../../ironclaw_first_party_extensions/assets/gmail/manifest.toml");

    struct RecordingReactivation {
        calls: AtomicUsize,
        fail_with: Option<String>,
    }

    impl RecordingReactivation {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                fail_with: None,
            }
        }

        fn failing(reason: &str) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                fail_with: Some(reason.to_string()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ChannelConfigReactivation for RecordingReactivation {
        async fn reactivate_if_active(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<(), ProductWorkflowError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.fail_with {
                Some(reason) => Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: reason.clone(),
                }),
                None => Ok(()),
            }
        }
    }

    async fn installed_store(
        manifest_toml: &str,
        id: &str,
    ) -> Arc<FilesystemExtensionInstallationStore> {
        let store = Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
        let record = ExtensionManifestRecord::from_toml(
            manifest_toml,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("fixture manifest parses");
        let extension_id = ExtensionId::new(id).expect("extension id");
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(id.to_string()).expect("installation id"),
                    extension_id.clone(),
                    ExtensionActivationState::Installed,
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    chrono::Utc::now(),
                    ironclaw_extensions::InstallationOwner::Tenant,
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");
        store
    }

    fn test_scope() -> ResourceScope {
        ResourceScope::local_default(
            UserId::new("channel-config-owner").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope")
    }

    struct Fixture {
        service: ChannelConfigService,
        installation_store: Arc<FilesystemExtensionInstallationStore>,
        secrets: Arc<FilesystemSecretStore<ironclaw_filesystem::InMemoryBackend>>,
        scope: ResourceScope,
        reactivation: Arc<RecordingReactivation>,
        extension_id: ExtensionId,
    }

    async fn channel_fixture(reactivation: RecordingReactivation) -> Fixture {
        let installation_store = installed_store(CHANNEL_FIXTURE_MANIFEST, "acmechat").await;
        let secrets = Arc::new(FilesystemSecretStore::ephemeral());
        let scope = test_scope();
        let reactivation = Arc::new(reactivation);
        let service = ChannelConfigService::new(
            Arc::clone(&installation_store) as Arc<dyn ExtensionInstallationStore>,
            Arc::clone(&secrets) as Arc<dyn SecretStore>,
            scope.clone(),
            Arc::clone(&reactivation) as Arc<dyn ChannelConfigReactivation>,
        );
        Fixture {
            service,
            installation_store,
            secrets,
            scope,
            reactivation,
            extension_id: ExtensionId::new("acmechat").expect("extension id"),
        }
    }

    #[tokio::test]
    async fn non_channel_auth_credentials_resolve_from_manifest_admin_configuration() {
        let installation_store = installed_store(NON_CHANNEL_ADMIN_FIXTURE_MANIFEST, "gmail").await;
        let manifest = installation_store
            .get_manifest(&ExtensionId::new("gmail").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(manifest.resolved().channel.is_none());
        assert!(!manifest.resolved().admin_configuration.is_empty());

        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let secrets: Arc<dyn SecretStore> = Arc::new(FilesystemSecretStore::ephemeral());
        let admin = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                    filesystem,
                    crate::invocation_mount_view,
                ))),
                Arc::clone(&secrets),
                manifest.resolved().admin_configuration.clone(),
            )
            .unwrap(),
        );
        let scope = test_scope();
        let group = manifest.resolved().admin_configuration[0].group_id.clone();
        admin
            .replace(
                &scope,
                &group,
                &AdminConfigurationIdempotencyKey::new("gmail-admin-save").unwrap(),
                0,
                vec![
                    AdminConfigurationSubmittedValue {
                        handle: SecretHandle::new("google_oauth_client_id").unwrap(),
                        value: SecretMaterial::from("client-id".to_string()),
                    },
                    AdminConfigurationSubmittedValue {
                        handle: SecretHandle::new("google_oauth_client_secret").unwrap(),
                        value: SecretMaterial::from("client-secret".to_string()),
                    },
                ],
            )
            .await
            .unwrap();
        let service = ChannelConfigService::new(
            Arc::clone(&installation_store) as Arc<dyn ExtensionInstallationStore>,
            Arc::clone(&secrets),
            scope.clone(),
            Arc::new(RecordingReactivation::new()),
        )
        .with_admin_configuration(admin, scope);

        for (handle, expected) in [
            ("google_oauth_client_id", "client-id"),
            ("google_oauth_client_secret", "client-secret"),
        ] {
            let value = service
                .credential_handle_value(handle)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(secrecy::ExposeSecret::expose_secret(&value), expected);
        }
    }

    #[tokio::test]
    async fn save_rejects_unknown_field_handles_and_stores_nothing() {
        let fixture = channel_fixture(RecordingReactivation::new()).await;

        let error = fixture
            .service
            .save(
                &fixture.extension_id,
                vec![
                    (
                        "acmechat_public_url".to_string(),
                        "https://x.example".to_string(),
                    ),
                    ("bogus_handle".to_string(), "value".to_string()),
                ],
            )
            .await
            .expect_err("unknown handle is a typed error");
        assert_eq!(
            error,
            ChannelConfigError::UnknownField {
                handle: "bogus_handle".to_string()
            }
        );
        assert!(
            fixture
                .installation_store
                .channel_config(&fixture.extension_id)
                .await
                .expect("read config")
                .is_empty(),
            "a rejected save must store nothing"
        );
        assert_eq!(fixture.reactivation.calls(), 0);
    }

    #[tokio::test]
    async fn save_routes_secrets_to_the_scoped_store_and_status_reports_presence_only() {
        let fixture = channel_fixture(RecordingReactivation::new()).await;

        // Before any save: every field reports unprovided.
        let status = fixture
            .service
            .status(&fixture.extension_id)
            .await
            .expect("status");
        assert_eq!(status.len(), 3);
        assert!(status.iter().all(|field| !field.provided));

        fixture
            .service
            .save(
                &fixture.extension_id,
                vec![
                    ("acmechat_api_token".to_string(), "tok-123".to_string()),
                    (
                        "acmechat_public_url".to_string(),
                        "https://x.example".to_string(),
                    ),
                ],
            )
            .await
            .expect("save succeeds");

        // Non-secret value → durable installation config.
        assert_eq!(
            fixture
                .installation_store
                .channel_config(&fixture.extension_id)
                .await
                .expect("read config"),
            vec![(
                "acmechat_public_url".to_string(),
                "https://x.example".to_string()
            )],
            "secret values must never reach the durable installation config"
        );
        // Secret value → the scoped secret store under the manifest handle,
        // where the channel egress credential fallback resolves it.
        let handle = SecretHandle::new("acmechat_api_token").expect("handle");
        assert!(
            fixture
                .secrets
                .metadata(&fixture.scope, &handle)
                .await
                .expect("metadata")
                .is_some(),
            "the secret must land at the channel-egress scope under the declared handle"
        );

        let status = fixture
            .service
            .status(&fixture.extension_id)
            .await
            .expect("status");
        let by_handle = |handle: &str| {
            status
                .iter()
                .find(|field| field.handle == handle)
                .unwrap_or_else(|| panic!("status must include {handle}"))
        };
        assert!(by_handle("acmechat_api_token").provided);
        assert!(by_handle("acmechat_api_token").secret);
        assert!(!by_handle("acmechat_webhook_secret").provided);
        assert!(by_handle("acmechat_public_url").provided);
        assert!(!by_handle("acmechat_public_url").secret);
        assert_eq!(by_handle("acmechat_public_url").label, "Public URL");

        // Blank secret submissions leave stored material unchanged.
        fixture
            .service
            .save(
                &fixture.extension_id,
                vec![("acmechat_api_token".to_string(), "  ".to_string())],
            )
            .await
            .expect("blank secret save is a no-op");
        assert!(
            fixture
                .secrets
                .metadata(&fixture.scope, &handle)
                .await
                .expect("metadata")
                .is_some()
        );
    }

    #[tokio::test]
    async fn save_triggers_the_reactivate_cycle_only_when_something_changed() {
        let fixture = channel_fixture(RecordingReactivation::new()).await;

        fixture
            .service
            .save(
                &fixture.extension_id,
                vec![(
                    "acmechat_public_url".to_string(),
                    "https://x.example".to_string(),
                )],
            )
            .await
            .expect("save succeeds");
        assert_eq!(
            fixture.reactivation.calls(),
            1,
            "a changed save runs the §6.5 cycle (the port no-ops when inactive)"
        );

        // Re-submitting the identical non-secret value changes nothing.
        fixture
            .service
            .save(
                &fixture.extension_id,
                vec![(
                    "acmechat_public_url".to_string(),
                    "https://x.example".to_string(),
                )],
            )
            .await
            .expect("identical save succeeds");
        assert_eq!(fixture.reactivation.calls(), 1, "no change, no cycle");

        // A secret write always counts as a change (secrets cannot be diffed).
        fixture
            .service
            .save(
                &fixture.extension_id,
                vec![("acmechat_api_token".to_string(), "tok-123".to_string())],
            )
            .await
            .expect("secret save succeeds");
        assert_eq!(fixture.reactivation.calls(), 2);
    }

    #[tokio::test]
    async fn reactivation_failure_surfaces_as_a_typed_error() {
        let fixture =
            channel_fixture(RecordingReactivation::failing("activation hook failed")).await;

        let error = fixture
            .service
            .save(
                &fixture.extension_id,
                vec![(
                    "acmechat_public_url".to_string(),
                    "https://x.example".to_string(),
                )],
            )
            .await
            .expect_err("reactivation failure surfaces");
        match error {
            ChannelConfigError::Reactivation { reason } => {
                assert!(reason.contains("activation hook failed"), "{reason}");
            }
            other => panic!("expected Reactivation error, got {other:?}"),
        }
        // The new value IS stored (the record is left per §6.1 with a typed
        // error; the operator fixes the value and saves again).
        assert_eq!(
            fixture
                .installation_store
                .channel_config(&fixture.extension_id)
                .await
                .expect("read config")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn credential_handle_value_resolves_declared_fields_only() {
        let fixture = channel_fixture(RecordingReactivation::new()).await;

        // Declared but not yet saved: no value, not an error.
        assert!(
            fixture
                .service
                .credential_handle_value("acmechat_api_token")
                .await
                .expect("declared-but-unsaved lookup succeeds")
                .is_none()
        );

        fixture
            .service
            .save(
                &fixture.extension_id,
                vec![
                    ("acmechat_api_token".to_string(), "tok-123".to_string()),
                    (
                        "acmechat_public_url".to_string(),
                        "https://x.example".to_string(),
                    ),
                ],
            )
            .await
            .expect("save succeeds");

        // Secret field -> scoped secret store material.
        let secret = fixture
            .service
            .credential_handle_value("acmechat_api_token")
            .await
            .expect("secret lookup succeeds")
            .expect("stored secret resolves");
        assert_eq!(secrecy::ExposeSecret::expose_secret(&secret), "tok-123");

        // Non-secret field -> durable installation config value.
        let non_secret = fixture
            .service
            .credential_handle_value("acmechat_public_url")
            .await
            .expect("non-secret lookup succeeds")
            .expect("stored value resolves");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&non_secret),
            "https://x.example"
        );

        assert_eq!(
            fixture
                .service
                .non_secret_handle_value(
                    &SecretHandle::new("acmechat_public_url").expect("config handle"),
                )
                .await
                .expect("non-secret URL-bound lookup succeeds")
                .as_deref(),
            Some("https://x.example")
        );
        assert!(
            fixture
                .service
                .non_secret_handle_value(
                    &SecretHandle::new("acmechat_api_token").expect("secret handle"),
                )
                .await
                .expect("secret URL-bound lookup fails closed without exposing material")
                .is_none()
        );

        // A handle no installed manifest declares resolves to nothing —
        // the auth engine falls through to its not-configured path.
        assert!(
            fixture
                .service
                .credential_handle_value("undeclared_handle")
                .await
                .expect("undeclared lookup succeeds")
                .is_none()
        );
    }

    #[tokio::test]
    async fn extensions_without_a_channel_surface_have_nothing_to_configure() {
        let installation_store = installed_store(TOOLS_ONLY_FIXTURE_MANIFEST, "zephyrite").await;
        let secrets = Arc::new(FilesystemSecretStore::ephemeral());
        let reactivation = Arc::new(RecordingReactivation::new());
        let service = ChannelConfigService::new(
            installation_store as Arc<dyn ExtensionInstallationStore>,
            secrets as Arc<dyn SecretStore>,
            test_scope(),
            reactivation as Arc<dyn ChannelConfigReactivation>,
        );
        let extension_id = ExtensionId::new("zephyrite").expect("extension id");

        assert!(
            service
                .status(&extension_id)
                .await
                .expect("status")
                .is_empty()
        );
        let error = service
            .save(
                &extension_id,
                vec![("anything".to_string(), "value".to_string())],
            )
            .await
            .expect_err("no declared fields admits no values");
        assert!(matches!(error, ChannelConfigError::UnknownField { .. }));

        let missing = ExtensionId::new("ghost").expect("extension id");
        let error = service
            .status(&missing)
            .await
            .expect_err("uninstalled extension is a typed error");
        assert!(matches!(error, ChannelConfigError::NotInstalled { .. }));
    }
}
