//! Generic shared-channel admission over manifest-declared administrator
//! configuration (extension-runtime §5.3).
//!
//! A channel extension opts into shared-conversation admission by declaring
//! non-secret `[admin_configuration]` fields with the handle-suffix convention
//! `*_allowed_channels` / `*_subject_routes` (or the bare names). When either
//! field is declared, the generic channel host assembly installs
//! [`AdminConfigurationSubjectRouteResolver`] on the extension's installation
//! scope and requires a configured route for every shared conversation —
//! unrouted shared conversations fail closed instead of falling to the
//! default subject.
//!
//! The two values are administrator-configured JSON:
//! - `*_subject_routes`: object mapping external conversation id to the
//!   subject user id turns in that conversation run as (explicit routes win).
//! - `*_allowed_channels`: array of external conversation ids admitted with
//!   a host-derived managed subject (`user:{extension_id}-channel:{sha16}` —
//!   the exact scheme the retired lane's route store derived, so folded
//!   deployments keep their managed-subject value shape).
//!
//! Reads are per-request through [`ComposedExtensionAdminConfigurationResolver`], so an authorized
//! Admin Configuration update takes effect without a route rebuild.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ExtensionAdminConfigurationDescriptor;
use ironclaw_host_api::{ExtensionId, TenantId, UserId};
use ironclaw_product::{AdapterInstallationId, ProductAdapterId};
use ironclaw_product::{
    ProductConversationSubjectRouteResolutionRequest, ProductConversationSubjectRouteResolver,
    ProductWorkflowError,
};
use sha2::{Digest, Sha256};

use crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver;

const ALLOWED_CHANNELS_FIELD: &str = "allowed_channels";
const SUBJECT_ROUTES_FIELD: &str = "subject_routes";

/// Handle-suffix convention shared with the connection-scoping claims:
/// `{name}` or `*_{name}` declares the admission field.
pub(crate) fn handle_declares_field(handle: &str, name: &str) -> bool {
    handle == name
        || handle
            .strip_suffix(name)
            .is_some_and(|prefix| prefix.ends_with('_'))
}

/// The admission config handles one extension's administrator schema declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SharedChannelAdmissionHandles {
    pub(crate) allowed_channels: Option<String>,
    pub(crate) subject_routes: Option<String>,
}

impl SharedChannelAdmissionHandles {
    pub(crate) fn declared(&self) -> bool {
        self.allowed_channels.is_some() || self.subject_routes.is_some()
    }
}

/// Scan the manifest's administrator configuration descriptors for the
/// admission handles (non-secret fields only — admission config is operator
/// routing data, never secret material).
pub(crate) fn shared_channel_admission_handles(
    descriptors: &[ExtensionAdminConfigurationDescriptor],
) -> SharedChannelAdmissionHandles {
    let find = |name: &str| {
        descriptors
            .iter()
            .flat_map(|descriptor| &descriptor.fields)
            .filter(|field| !field.secret)
            .find(|field| handle_declares_field(field.handle.as_str(), name))
            .map(|field| field.handle.as_str().to_string())
    };
    SharedChannelAdmissionHandles {
        allowed_channels: find(ALLOWED_CHANNELS_FIELD),
        subject_routes: find(SUBJECT_ROUTES_FIELD),
    }
}

/// The host-derived managed subject for one allowed shared conversation:
/// `user:{extension_id}-channel:{first 16 digest bytes as hex}` over
/// `sha256(tenant \0 installation \0 space \0 conversation)` — ported
/// unchanged from the retired lane's route store so folded deployments
/// derive the same subject ids for the same channels.
pub(crate) fn managed_channel_subject_user_id(
    extension_id: &str,
    tenant_id: &TenantId,
    installation_id: &AdapterInstallationId,
    space_id: Option<&str>,
    conversation_id: &str,
) -> Result<UserId, String> {
    let mut hasher = Sha256::new();
    hasher.update(tenant_id.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(installation_id.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(space_id.unwrap_or_default().as_bytes());
    hasher.update(b"\0");
    hasher.update(conversation_id.as_bytes());
    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        write!(&mut suffix, "{byte:02x}")
            .map_err(|error| format!("managed subject render failed: {error}"))?;
    }
    UserId::new(format!("user:{extension_id}-channel:{suffix}"))
        .map_err(|error| format!("managed subject id invalid: {error}"))
}

/// The default generic subject-route resolver: explicit `*_subject_routes`
/// entries win; `*_allowed_channels` entries admit with the managed derived
/// subject; everything else resolves to no route (which the assembly's
/// require-configured-route policy fails closed).
pub(crate) struct AdminConfigurationSubjectRouteResolver {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    tenant_id: TenantId,
    extension_id: ExtensionId,
    handles: SharedChannelAdmissionHandles,
    admin_configuration_resolver: Arc<ComposedExtensionAdminConfigurationResolver>,
}

impl AdminConfigurationSubjectRouteResolver {
    pub(crate) fn new(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        tenant_id: TenantId,
        extension_id: ExtensionId,
        handles: SharedChannelAdmissionHandles,
        admin_configuration_resolver: Arc<ComposedExtensionAdminConfigurationResolver>,
    ) -> Self {
        Self {
            adapter_id,
            installation_id,
            tenant_id,
            extension_id,
            handles,
            admin_configuration_resolver,
        }
    }

    async fn config_value(&self, handle: &str) -> Result<Option<String>, ProductWorkflowError> {
        self.admin_configuration_resolver
            .non_secret_value(&self.extension_id, handle)
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("channel admission config unavailable: {error}"),
            })
    }
}

impl std::fmt::Debug for AdminConfigurationSubjectRouteResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdminConfigurationSubjectRouteResolver")
            .field("extension_id", &self.extension_id)
            .field("handles", &self.handles)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ProductConversationSubjectRouteResolver for AdminConfigurationSubjectRouteResolver {
    async fn resolve_product_conversation_subject_route(
        &self,
        request: ProductConversationSubjectRouteResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if request.adapter_id != self.adapter_id || request.installation_id != self.installation_id
        {
            return Ok(None);
        }
        let conversation_id = request.route_key.conversation_id();
        if let Some(handle) = &self.handles.subject_routes
            && let Some(raw) = self.config_value(handle).await?
        {
            match serde_json::from_str::<BTreeMap<String, String>>(&raw) {
                Ok(routes) => {
                    if let Some(subject) = routes.get(conversation_id) {
                        return UserId::new(subject.clone()).map(Some).map_err(|error| {
                            ProductWorkflowError::InvalidBindingRequest {
                                reason: format!("configured subject route is invalid: {error}"),
                            }
                        });
                    }
                }
                // Malformed operator JSON fails closed: no route resolves
                // until the value is fixed through the configure surface.
                Err(error) => {
                    tracing::warn!(
                        target = "ironclaw::reborn::channel_host",
                        extension_id = %self.extension_id,
                        handle = %handle,
                        %error,
                        "subject-route config value is not a JSON object; treating as no routes"
                    );
                }
            }
        }
        if let Some(handle) = &self.handles.allowed_channels
            && let Some(raw) = self.config_value(handle).await?
        {
            match serde_json::from_str::<Vec<String>>(&raw) {
                Ok(allowed) => {
                    if allowed.iter().any(|entry| entry == conversation_id) {
                        return managed_channel_subject_user_id(
                            self.extension_id.as_str(),
                            &self.tenant_id,
                            &self.installation_id,
                            request.route_key.space_id(),
                            conversation_id,
                        )
                        .map(Some)
                        .map_err(|reason| ProductWorkflowError::InvalidBindingRequest { reason });
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target = "ironclaw::reborn::channel_host",
                        extension_id = %self.extension_id,
                        handle = %handle,
                        %error,
                        "allowed-channel config value is not a JSON array; treating as empty"
                    );
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_extension_host::{AdminConfigurationService, FilesystemAdminConfigurationStore};
    use ironclaw_extensions::{ExtensionManifestRecord, ManifestSource};
    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{InvocationId, ResourceScope};
    use ironclaw_product::ProductConversationRouteKey;
    use ironclaw_secrets::{FilesystemSecretStore, SecretStore};

    use super::*;
    use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

    /// Invented channel extension declaring the admission fields by the
    /// handle-suffix convention.
    const ADMISSION_FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "vendorx"
name = "VendorX"
version = "0.1.0"
description = "shared-channel admission fixture"
trust = "first_party_requested"

[admin_configuration]
group_id = "extension.vendorx"
display_name = "VendorX deployment configuration"
fields = [
  { handle = "vendorx_webhook_secret", label = "Webhook secret", secret = true, required = false },
  { handle = "vendorx_allowed_channels", label = "Allowed channels", secret = false, required = false },
  { handle = "vendorx_subject_routes", label = "Subject routes", secret = false, required = false },
]

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

[channel.presentation]
supports_markdown = false
supports_threads = false
"#;

    const TENANT: &str = "tenant-alpha";
    const INSTALLATION: &str = "vendorx-install-1";

    struct Fixture {
        resolver: AdminConfigurationSubjectRouteResolver,
        admin_configuration_resolver: Arc<ComposedExtensionAdminConfigurationResolver>,
    }

    async fn fixture() -> Fixture {
        let record = ExtensionManifestRecord::from_toml(
            ADMISSION_FIXTURE_MANIFEST,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("fixture manifest parses");
        let manifest = Arc::new(record.resolved().clone());
        let extension_id = ExtensionId::new("vendorx").expect("extension id");
        let scope = ResourceScope::local_default(
            UserId::new("operator").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let secrets: Arc<dyn SecretStore> = Arc::new(FilesystemSecretStore::ephemeral());
        let admin = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                    filesystem,
                    crate::invocation_mount_view,
                ))),
                secrets,
                manifest.admin_configuration.clone(),
            )
            .expect("admin configuration service"),
        );
        let admin_configuration_resolver = Arc::new(
            ComposedExtensionAdminConfigurationResolver::new(admin, scope, [Arc::clone(&manifest)]),
        );
        let handles = SharedChannelAdmissionHandles {
            allowed_channels: Some("vendorx_allowed_channels".to_string()),
            subject_routes: Some("vendorx_subject_routes".to_string()),
        };
        let resolver = AdminConfigurationSubjectRouteResolver::new(
            ProductAdapterId::new("vendorx").expect("adapter id"),
            AdapterInstallationId::new(INSTALLATION).expect("installation id"),
            TenantId::new(TENANT).expect("tenant"),
            extension_id.clone(),
            handles,
            Arc::clone(&admin_configuration_resolver),
        );
        Fixture {
            resolver,
            admin_configuration_resolver,
        }
    }

    fn request(
        adapter: &str,
        installation: &str,
        space: Option<&str>,
        conversation: &str,
    ) -> ProductConversationSubjectRouteResolutionRequest {
        ProductConversationSubjectRouteResolutionRequest {
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
            .admin_configuration_resolver
            .configure_admin_group_for_test(
                "extension.vendorx",
                vec![(handle.to_string(), value.to_string())],
            )
            .await
            .expect("config save");
    }

    #[test]
    fn admission_handles_follow_the_suffix_convention_on_non_secret_fields() {
        let record = ExtensionManifestRecord::from_toml(
            ADMISSION_FIXTURE_MANIFEST,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("fixture manifest parses");
        let handles = shared_channel_admission_handles(&record.resolved().admin_configuration);
        assert_eq!(
            handles,
            SharedChannelAdmissionHandles {
                allowed_channels: Some("vendorx_allowed_channels".to_string()),
                subject_routes: Some("vendorx_subject_routes".to_string()),
            }
        );
        assert!(handles.declared());
        // A channel without the convention declares nothing.
        assert!(!shared_channel_admission_handles(&[]).declared());
        assert!(handle_declares_field(
            "allowed_channels",
            "allowed_channels"
        ));
        assert!(!handle_declares_field(
            "disallowed_channels",
            "allowed_channels"
        ));
    }

    #[tokio::test]
    async fn unconfigured_admission_resolves_no_route() {
        let fixture = fixture().await;
        let resolved = fixture
            .resolver
            .resolve_product_conversation_subject_route(request(
                "vendorx",
                INSTALLATION,
                Some("S-1"),
                "C777",
            ))
            .await
            .expect("resolution succeeds");
        assert_eq!(resolved, None, "no saved config admits nothing");
    }

    #[tokio::test]
    async fn explicit_subject_routes_win_over_allowed_channels() {
        let fixture = fixture().await;
        save(&fixture, "vendorx_allowed_channels", r#"["C777","C888"]"#).await;
        save(
            &fixture,
            "vendorx_subject_routes",
            r#"{"C888":"user:ops-agent"}"#,
        )
        .await;

        let explicit = fixture
            .resolver
            .resolve_product_conversation_subject_route(request(
                "vendorx",
                INSTALLATION,
                Some("S-1"),
                "C888",
            ))
            .await
            .expect("resolution succeeds")
            .expect("explicit route resolves");
        assert_eq!(explicit.as_str(), "user:ops-agent");

        let managed = fixture
            .resolver
            .resolve_product_conversation_subject_route(request(
                "vendorx",
                INSTALLATION,
                Some("S-1"),
                "C777",
            ))
            .await
            .expect("resolution succeeds")
            .expect("allowed channel resolves a managed subject");
        let expected = managed_channel_subject_user_id(
            "vendorx",
            &TenantId::new(TENANT).expect("tenant"),
            &AdapterInstallationId::new(INSTALLATION).expect("installation"),
            Some("S-1"),
            "C777",
        )
        .expect("derivation");
        assert_eq!(managed, expected);
        assert!(managed.as_str().starts_with("user:vendorx-channel:"));
        assert_eq!(
            managed.as_str().len(),
            "user:vendorx-channel:".len() + 32,
            "managed subject carries a 16-byte hex digest"
        );

        // A channel in neither value stays unrouted.
        assert_eq!(
            fixture
                .resolver
                .resolve_product_conversation_subject_route(request(
                    "vendorx",
                    INSTALLATION,
                    Some("S-1"),
                    "C999",
                ))
                .await
                .expect("resolution succeeds"),
            None
        );
    }

    #[tokio::test]
    async fn managed_subject_derivation_matches_the_retired_lane_scheme() {
        // Independent recomputation of the ported derivation:
        // sha256(tenant \0 installation \0 space \0 conversation), first 16
        // digest bytes hex-encoded under `user:{extension}-channel:`.
        let mut hasher = Sha256::new();
        for (index, part) in [TENANT, INSTALLATION, "S-1", "C777"].iter().enumerate() {
            if index > 0 {
                hasher.update(b"\0");
            }
            hasher.update(part.as_bytes());
        }
        let digest = hasher.finalize();
        let expected_suffix = digest
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let derived = managed_channel_subject_user_id(
            "vendorx",
            &TenantId::new(TENANT).expect("tenant"),
            &AdapterInstallationId::new(INSTALLATION).expect("installation"),
            Some("S-1"),
            "C777",
        )
        .expect("derivation");
        assert_eq!(
            derived.as_str(),
            format!("user:vendorx-channel:{expected_suffix}")
        );
    }

    #[tokio::test]
    async fn malformed_config_json_fails_closed() {
        let fixture = fixture().await;
        save(&fixture, "vendorx_allowed_channels", "not-json").await;
        save(&fixture, "vendorx_subject_routes", "[]").await;

        assert_eq!(
            fixture
                .resolver
                .resolve_product_conversation_subject_route(request(
                    "vendorx",
                    INSTALLATION,
                    Some("S-1"),
                    "C777",
                ))
                .await
                .expect("resolution succeeds"),
            None,
            "malformed operator JSON must never admit"
        );
    }

    #[tokio::test]
    async fn foreign_adapter_or_installation_resolves_nothing() {
        let fixture = fixture().await;
        save(&fixture, "vendorx_allowed_channels", r#"["C777"]"#).await;

        for request in [
            request("othervendor", INSTALLATION, Some("S-1"), "C777"),
            request("vendorx", "other-install", Some("S-1"), "C777"),
        ] {
            assert_eq!(
                fixture
                    .resolver
                    .resolve_product_conversation_subject_route(request)
                    .await
                    .expect("resolution succeeds"),
                None
            );
        }
    }

    #[tokio::test]
    async fn config_saves_take_effect_per_request() {
        let fixture = fixture().await;
        let request_c7 = || request("vendorx", INSTALLATION, Some("S-1"), "C777");
        assert_eq!(
            fixture
                .resolver
                .resolve_product_conversation_subject_route(request_c7())
                .await
                .expect("resolution succeeds"),
            None
        );
        save(&fixture, "vendorx_allowed_channels", r#"["C777"]"#).await;
        assert!(
            fixture
                .resolver
                .resolve_product_conversation_subject_route(request_c7())
                .await
                .expect("resolution succeeds")
                .is_some(),
            "a configure save admits on the next request with no rebuild"
        );
    }
}
