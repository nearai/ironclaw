//! Generic shared-channel admission over `[channel.config]`
//! (extension-runtime §5.3).
//!
//! A channel extension opts into shared-conversation admission by declaring a
//! non-secret `[channel.config]` field with the handle-suffix convention
//! `*_allowed_channels` (or the bare name). When the field is declared, the
//! generic channel host assembly installs [`ChannelConfigSharedAdmission`] on
//! the extension's installation scope; shared conversations the saved value
//! does not list fail closed.
//!
//! The value is operator-saved JSON: an array of external conversation ids
//! connected to this deployment. There is no subject half any more — a run
//! acts as the user who invoked it, so admission answers only "is this
//! conversation connected", never "whom does it run as". The retired
//! `*_subject_routes` handle is deliberately not read: legacy saved values
//! are inert, and a deployment that admitted conversations only through
//! subject routes re-admits them through `*_allowed_channels`.
//!
//! Reads are per-request through [`ChannelConfigService`]: a configure save
//! takes effect on the next inbound admission with no route rebuild.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::recipe::RecipeSecretField;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::shared_admission::{
    SharedConversationAdmission, SharedConversationAdmissionRequest,
};

use crate::ChannelConfigService;
use crate::channel_config::ChannelConfigError;

const ALLOWED_CHANNELS_FIELD: &str = "allowed_channels";

/// Handle-suffix convention shared with the connection-scoping claims:
/// `{name}` or `*_{name}` declares the admission field.
pub fn handle_declares_field(handle: &str, name: &str) -> bool {
    handle == name
        || handle
            .strip_suffix(name)
            .is_some_and(|prefix| prefix.ends_with('_'))
}

/// Scan the manifest's `[channel.config]` field descriptors for the admission
/// handle an extension declares, if any (non-secret fields only — admission
/// config is operator routing data, never secret material).
pub fn shared_channel_admission_handle(fields: &[RecipeSecretField]) -> Option<String> {
    fields
        .iter()
        .filter(|field| !field.secret)
        .find(|field| handle_declares_field(field.handle.as_str(), ALLOWED_CHANNELS_FIELD))
        .map(|field| field.handle.as_str().to_string())
}

/// The generic admission resolver: a shared conversation is admitted iff the
/// operator-saved `*_allowed_channels` array lists its conversation id.
/// Everything else — no saved value, malformed value, unlisted conversation,
/// foreign adapter/installation — resolves to not-admitted, which the product
/// workflow fails closed.
pub struct ChannelConfigSharedAdmission {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    extension_id: ExtensionId,
    /// The manifest-declared `*_allowed_channels` handle. A resolver exists
    /// only for a channel that declares one — "installed but handle-less" is
    /// not a representable state.
    allowed_channels_handle: String,
    channel_config: Arc<ChannelConfigService>,
}

impl ChannelConfigSharedAdmission {
    pub fn new(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        extension_id: ExtensionId,
        allowed_channels_handle: String,
        channel_config: Arc<ChannelConfigService>,
    ) -> Self {
        Self {
            adapter_id,
            installation_id,
            extension_id,
            allowed_channels_handle,
            channel_config,
        }
    }

    async fn config_value(&self, handle: &str) -> Result<Option<String>, ProductOperationFailure> {
        self.channel_config
            .non_secret_value(&self.extension_id, handle)
            .await
            .map_err(channel_config_unavailable)
    }
}

/// A config-store failure is **transient**, never a rejection.
///
/// The distinction is load-bearing and is why this is a named function rather
/// than an inline closure: `Transient` projects to a retryable 503, while any
/// rejection variant would project to a permanent 4xx. Classifying an
/// unavailable store as permanent would make a shared channel look
/// mis-configured — and stay that way for the caller — during what is really a
/// storage blip. Naming it also makes the classification directly testable
/// without having to fault-inject the whole config service.
fn channel_config_unavailable(error: ChannelConfigError) -> ProductOperationFailure {
    ProductOperationFailure::Transient {
        reason: format!("channel admission config unavailable: {error}"),
    }
}

impl std::fmt::Debug for ChannelConfigSharedAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelConfigSharedAdmission")
            .field("extension_id", &self.extension_id)
            .field("allowed_channels_handle", &self.allowed_channels_handle)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SharedConversationAdmission for ChannelConfigSharedAdmission {
    async fn shared_conversation_admitted(
        &self,
        request: SharedConversationAdmissionRequest,
    ) -> Result<bool, ProductOperationFailure> {
        if request.adapter_id != self.adapter_id || request.installation_id != self.installation_id
        {
            return Ok(false);
        }
        let conversation_id = request.route_key.conversation_id();
        if let Some(raw) = self
            .config_value(self.allowed_channels_handle.as_str())
            .await?
        {
            match serde_json::from_str::<Vec<String>>(&raw) {
                Ok(allowed) => {
                    return Ok(allowed.iter().any(|entry| entry == conversation_id));
                }
                // Malformed operator JSON fails closed: nothing is admitted
                // until the value is fixed through the configure surface.
                Err(error) => {
                    tracing::warn!(
                        target: "ironclaw::reborn::channel_host",
                        extension_id = %self.extension_id,
                        handle = %self.allowed_channels_handle,
                        %error,
                        "allowed-channel config value is not a JSON array; treating as empty"
                    );
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_extension_registry::{
        ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStore,
        ExtensionInstallationStorePort, ExtensionManifestRecord, ExtensionManifestRef,
        ManifestSource,
    };
    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{
        host_port::HostPortCatalog,
        ids::{InvocationId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
    };
    use ironclaw_product_contracts::shared_admission::ProductConversationRouteKey;
    use ironclaw_product_contracts::surface::ProductSurfaceError;
    use ironclaw_secrets::{SecretStore, SecretStorePort};

    use super::*;
    use crate::{AdminConfigurationService, FilesystemAdminConfigurationStore};

    /// Invented channel extension declaring the admission field by the
    /// handle-suffix convention — plus the RETIRED `*_subject_routes` handle,
    /// kept in the fixture on purpose: a legacy deployment may still declare
    /// and hold saved subject-route values, and admission must treat them as
    /// inert rather than admitting their conversations.
    const ADMISSION_FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "vendorx"
name = "VendorX"
version = "0.1.0"
description = "shared-channel admission fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "vendorx.extension/v1"

[channel]
id = "messages"
display_name = "VendorX messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "vendorx_webhook_secret"
header = "X-VendorX-Secret"

[admin_configuration]
group_id = "vendorx.channel"
display_name = "VendorX channel"
fields = [
  { handle = "vendorx_webhook_secret", label = "Webhook secret", secret = true },
  { handle = "vendorx_allowed_channels", label = "Allowed channels", secret = false },
  { handle = "vendorx_subject_routes", label = "Subject routes (retired)", secret = false },
]

[channel.presentation]
supports_markdown = false
supports_threads = false
"#;

    const INSTALLATION: &str = "vendorx-install-1";

    struct Fixture {
        resolver: ChannelConfigSharedAdmission,
        channel_config: Arc<ChannelConfigService>,
        extension_id: ExtensionId,
    }

    struct NoopReactivation;

    #[async_trait]
    impl crate::ChannelConfigReactivation for NoopReactivation {
        async fn reactivate_if_active(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<(), crate::ChannelConfigReactivationError> {
            Ok(())
        }
    }

    async fn filesystem_installation_store_for_test() -> ExtensionInstallationStore {
        ExtensionInstallationStore::load_at(
            Arc::new(InMemoryBackend::new()),
            VirtualPath::new("/system/extensions/.installations/test").expect("valid test path"),
            HostPortCatalog::empty(),
            product_extension_host_api_contract_registry().expect("extension host API contracts"),
        )
        .await
        .expect("filesystem extension installation store")
    }

    fn product_extension_host_api_contract_registry() -> Result<
        ironclaw_extension_registry::HostApiContractRegistry,
        ironclaw_extension_registry::ManifestV2Error,
    > {
        let mut registry = ironclaw_extension_registry::default_host_api_contract_registry()?;
        ironclaw_extension_registry::host_api::product_adapter::register_product_adapter_host_api_contract(
            &mut registry,
        )
        .map_err(|error| ironclaw_extension_registry::ManifestV2Error::Invalid {
            reason: format!("product adapter host API contract registration failed: {error}"),
        })?;
        Ok(registry)
    }

    async fn fixture() -> Fixture {
        let store = Arc::new(filesystem_installation_store_for_test().await);
        let record = ExtensionManifestRecord::from_toml(
            ADMISSION_FIXTURE_MANIFEST,
            ManifestSource::HostBundled,
            &ironclaw_host_api::host_port::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
            None,
        )
        .expect("fixture manifest parses");
        let extension_id = ExtensionId::new("vendorx").expect("extension id");
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(INSTALLATION.to_string())
                        .expect("installation id"),
                    extension_id.clone(),
                    ExtensionManifestRef::new(extension_id.clone(), None),
                    Vec::new(),
                    chrono::Utc::now(),
                    ironclaw_extension_registry::InstallationOwner::Tenant,
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");
        let scope = ResourceScope::local_default(
            UserId::new("operator").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope");
        let secrets = Arc::new(SecretStore::ephemeral());
        let admin_secrets: Arc<dyn SecretStorePort> =
            Arc::clone(&secrets) as Arc<dyn SecretStorePort>;
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let manifest = store
            .get_manifest(&extension_id)
            .await
            .expect("manifest read")
            .expect("manifest installed");
        let admin = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                    filesystem,
                    |_scope| {
                        MountView::new(vec![MountGrant::new(
                            MountAlias::new("/extension-admin-configuration")
                                .expect("valid mount alias"),
                            VirtualPath::new("/tenants/test/shared/admin-configuration")
                                .expect("valid virtual path"),
                            MountPermissions::read_write_list_delete(),
                        )])
                    },
                ))),
                admin_secrets,
                manifest.resolved().admin_configuration.clone(),
            )
            .expect("admin configuration service"),
        );
        let channel_config = Arc::new(
            ChannelConfigService::new(
                store,
                Arc::clone(&secrets) as Arc<dyn SecretStorePort>,
                scope.clone(),
                Arc::new(NoopReactivation),
            )
            .with_admin_configuration(admin, scope),
        );
        let resolver = ChannelConfigSharedAdmission::new(
            ProductAdapterId::new("vendorx").expect("adapter id"),
            AdapterInstallationId::new(INSTALLATION).expect("installation id"),
            extension_id.clone(),
            "vendorx_allowed_channels".to_string(),
            Arc::clone(&channel_config),
        );
        Fixture {
            resolver,
            channel_config,
            extension_id,
        }
    }

    fn request(
        adapter: &str,
        installation: &str,
        space: Option<&str>,
        conversation: &str,
    ) -> SharedConversationAdmissionRequest {
        SharedConversationAdmissionRequest {
            adapter_id: ProductAdapterId::new(adapter).expect("adapter id"),
            installation_id: AdapterInstallationId::new(installation).expect("installation id"),
            route_key: ProductConversationRouteKey::new(
                space.map(str::to_string),
                conversation.to_string(),
            )
            .expect("route key"),
        }
    }

    async fn save(fixture: &Fixture, handle: &str, value: &str) {
        fixture
            .channel_config
            .save(
                &fixture.extension_id,
                vec![(handle.to_string(), value.to_string())],
            )
            .await
            .expect("config save");
    }

    /// Every way the config store can fail is transient, so an unavailable
    /// store yields a retryable 503 rather than a permanent rejection that
    /// would leave a correctly-configured channel looking broken to its caller.
    ///
    /// Driven directly rather than by fault-injecting the config service — the
    /// reason the mapping is a named function. Asserted through the projection
    /// the caller actually sees, not just the discriminant, so a variant swap
    /// that kept the enum shape but changed the status still fails.
    #[test]
    fn every_config_store_failure_is_transient_and_projects_to_a_retryable_503() {
        for error in [
            ChannelConfigError::NotInstalled {
                extension_id: "channel-fixture".to_string(),
            },
            ChannelConfigError::UnknownField {
                handle: "allowed_channels".to_string(),
            },
            ChannelConfigError::Storage {
                reason: "backend offline".to_string(),
            },
            ChannelConfigError::Reactivation {
                reason: "restart failed".to_string(),
            },
        ] {
            let mapped = channel_config_unavailable(error.clone());
            assert!(
                matches!(mapped, ProductOperationFailure::Transient { .. }),
                "{error:?} must be transient, got {mapped:?}"
            );

            let projected: ProductSurfaceError = mapped.into();
            assert_eq!(projected.status_code, 503, "status for {error:?}");
            assert!(projected.retryable, "retryable for {error:?}");
        }
    }

    #[test]
    fn admission_handles_follow_the_suffix_convention_on_non_secret_fields() {
        let record = ExtensionManifestRecord::from_toml(
            ADMISSION_FIXTURE_MANIFEST,
            ManifestSource::HostBundled,
            &ironclaw_host_api::host_port::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
            None,
        )
        .expect("fixture manifest parses");
        let fields = record
            .resolved()
            .admin_configuration
            .iter()
            .flat_map(|descriptor| descriptor.fields.iter())
            .map(|field| RecipeSecretField {
                handle: field.handle.clone(),
                label: field.label.clone(),
                secret: field.secret,
            })
            .collect::<Vec<_>>();
        let handle = shared_channel_admission_handle(&fields);
        assert_eq!(handle.as_deref(), Some("vendorx_allowed_channels"));
        // A secret field never declares admission config, whatever its name.
        let secret_only = [RecipeSecretField {
            handle: ironclaw_host_api::ids::SecretHandle::new("vendorx_allowed_channels")
                .expect("valid handle"),
            label: "secret impostor".to_string(),
            secret: true,
        }];
        assert_eq!(shared_channel_admission_handle(&secret_only), None);
    }

    #[tokio::test]
    async fn unconfigured_admission_admits_nothing() {
        let fixture = fixture().await;
        let admitted = fixture
            .resolver
            .shared_conversation_admitted(request("vendorx", INSTALLATION, Some("S1"), "C777"))
            .await
            .expect("admission resolves");
        assert!(!admitted, "no saved config admits nothing");
    }

    #[tokio::test]
    async fn allowed_channels_admit_exactly_the_listed_conversations() {
        let fixture = fixture().await;
        save(&fixture, "vendorx_allowed_channels", r#"["C777","C888"]"#).await;

        for (conversation, expected) in [("C777", true), ("C888", true), ("C999", false)] {
            let admitted = fixture
                .resolver
                .shared_conversation_admitted(request(
                    "vendorx",
                    INSTALLATION,
                    Some("S1"),
                    conversation,
                ))
                .await
                .expect("admission resolves");
            assert_eq!(admitted, expected, "conversation {conversation}");
        }
    }

    /// Legacy `*_subject_routes` values are INERT: they neither admit their
    /// conversations nor scope anyone. A deployment that admitted
    /// conversations only through subject routes re-admits them through
    /// `*_allowed_channels`.
    #[tokio::test]
    async fn legacy_subject_route_values_do_not_admit() {
        let fixture = fixture().await;
        save(
            &fixture,
            "vendorx_subject_routes",
            r#"{"C777":"user:someone"}"#,
        )
        .await;

        let admitted = fixture
            .resolver
            .shared_conversation_admitted(request("vendorx", INSTALLATION, Some("S1"), "C777"))
            .await
            .expect("admission resolves");
        assert!(
            !admitted,
            "a saved subject-route value must not admit its conversation"
        );
    }

    #[tokio::test]
    async fn malformed_config_json_fails_closed() {
        let fixture = fixture().await;
        save(&fixture, "vendorx_allowed_channels", "not-json").await;

        let admitted = fixture
            .resolver
            .shared_conversation_admitted(request("vendorx", INSTALLATION, Some("S1"), "C777"))
            .await
            .expect("malformed config is not an error");
        assert!(!admitted, "malformed config admits nothing");
    }

    #[tokio::test]
    async fn foreign_adapter_or_installation_admits_nothing() {
        let fixture = fixture().await;
        save(&fixture, "vendorx_allowed_channels", r#"["C777"]"#).await;

        for req in [
            request("vendory", INSTALLATION, Some("S1"), "C777"),
            request("vendorx", "vendorx-install-2", Some("S1"), "C777"),
        ] {
            let admitted = fixture
                .resolver
                .shared_conversation_admitted(req)
                .await
                .expect("admission resolves");
            assert!(!admitted);
        }
    }

    #[tokio::test]
    async fn config_saves_take_effect_per_request() {
        let fixture = fixture().await;
        let req = request("vendorx", INSTALLATION, Some("S1"), "C777");
        assert!(
            !fixture
                .resolver
                .shared_conversation_admitted(req.clone())
                .await
                .expect("admission resolves")
        );
        save(&fixture, "vendorx_allowed_channels", r#"["C777"]"#).await;
        assert!(
            fixture
                .resolver
                .shared_conversation_admitted(req)
                .await
                .expect("admission resolves"),
            "a configure save admits on the next request with no rebuild"
        );
    }
}
