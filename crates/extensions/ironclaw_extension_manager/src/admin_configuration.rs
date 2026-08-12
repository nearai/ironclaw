//! Manifest-driven tenant administrator configuration adapters.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_assistant::{
    ADMIN_CONFIGURATION_VIEW, RebornAdminConfigurationField, RebornAdminConfigurationGroup,
    RebornAdminConfigurationListResponse, RebornAdminConfigurationUse,
};
use ironclaw_extension_host::{AdminConfigurationGroupState, AdminConfigurationService};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{ids::InvocationId, resource::ResourceScope};
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
};
use ironclaw_product_contracts::views::{RebornViewDescriptor, RebornViewPage, RebornViewProvider};

use ironclaw_extension_host::AdminConfigurationCatalogUse;

pub type ComposedAdminConfigurationService =
    AdminConfigurationService<dyn RootFilesystem, dyn ironclaw_secrets::SecretStorePort>;
pub type ComposedExtensionAdminConfigurationResolver =
    ironclaw_extension_host::ChannelConfigService;

#[derive(Clone, Default)]
pub struct AdminConfigurationViewProvider {
    parts: Option<Arc<AdminConfigurationViewParts>>,
}

struct AdminConfigurationViewParts {
    service: Arc<ComposedAdminConfigurationService>,
    uses: Arc<Vec<AdminConfigurationCatalogUse>>,
    installation_store: Arc<dyn ironclaw_extension_registry::ExtensionInstallationStorePort>,
}

impl AdminConfigurationViewProvider {
    pub fn new(
        service: Arc<ComposedAdminConfigurationService>,
        uses: Vec<AdminConfigurationCatalogUse>,
        installation_store: Arc<dyn ironclaw_extension_registry::ExtensionInstallationStorePort>,
    ) -> Self {
        Self {
            parts: Some(Arc::new(AdminConfigurationViewParts {
                service,
                uses: Arc::new(uses),
                installation_store,
            })),
        }
    }
}

#[async_trait]
impl RebornViewProvider for AdminConfigurationViewProvider {
    fn descriptor(&self) -> RebornViewDescriptor {
        ADMIN_CONFIGURATION_VIEW
    }

    async fn query(
        &self,
        caller: ProductSurfaceCaller,
        params: serde_json::Value,
        cursor: Option<String>,
    ) -> Result<RebornViewPage, ProductSurfaceError> {
        if !caller.operator_config {
            return Err(forbidden());
        }
        if params != serde_json::json!({}) || cursor.is_some() {
            return Err(invalid_request());
        }
        let Some(parts) = &self.parts else {
            return Err(service_error(
                ProductSurfaceErrorCode::Unavailable,
                ProductSurfaceErrorKind::ServiceUnavailable,
                503,
            ));
        };
        let scope = caller_scope(&caller);
        let states = parts
            .service
            .list(&scope)
            .await
            .map_err(map_admin_configuration_error)?;
        let installed = parts
            .installation_store
            .list_installations()
            .await
            .map_err(installed_extension_listing_error)?
            .into_iter()
            .map(|installation| installation.extension_id().as_str().to_string())
            .collect::<BTreeSet<_>>();
        let groups = states
            .into_iter()
            .map(|state| render_group(state, &parts.uses, &installed))
            .collect();
        let payload = serde_json::to_value(RebornAdminConfigurationListResponse { groups })
            .map_err(ProductSurfaceError::internal_from)?;
        Ok(RebornViewPage {
            payload,
            next_cursor: None,
        })
    }
}

fn render_group(
    state: AdminConfigurationGroupState,
    uses: &[AdminConfigurationCatalogUse],
    installed: &BTreeSet<String>,
) -> RebornAdminConfigurationGroup {
    let group_id = state.group_id.as_str().to_string();
    RebornAdminConfigurationGroup {
        used_by: uses
            .iter()
            .filter(|usage| usage.descriptor.group_id == state.group_id)
            .map(|usage| RebornAdminConfigurationUse {
                package_id: usage.package_id.clone(),
                display_name: usage.display_name.clone(),
                installed: installed.contains(&usage.package_id),
            })
            .collect(),
        group_id,
        display_name: state.display_name,
        description: state.description,
        revision: state.revision,
        complete: state.complete,
        fields: state
            .fields
            .into_iter()
            .map(|field| RebornAdminConfigurationField {
                handle: field.handle.as_str().to_string(),
                label: field.label,
                required: field.required,
                provided: field.provided,
                // Defense in depth, same as the capability handler's
                // `render_state`: the service already redacts secret values,
                // but this view must not depend on that staying true.
                value: if field.secret { None } else { field.value },
                secret: field.secret,
            })
            .collect(),
    }
}

pub fn caller_scope(caller: &ProductSurfaceCaller) -> ResourceScope {
    ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// The installed-extension read is what decorates each group with "which
/// packages use this, and are they installed" — a failure there is an internal
/// error, never an empty `used_by` list.
///
/// `ExtensionInstallationStorePort` is filesystem-backed, so its own text can
/// name a mount path (`StoreUnavailable { reason }` embeds the backend's
/// message verbatim). It is therefore kept out of the value handed to
/// `internal_from` — which logs whatever it is given at `error!` — and recorded
/// here instead, at `debug!`: `info!`/`warn!` corrupt the REPL/TUI, and this is
/// an internal diagnostic rather than operator-facing status. The caller's
/// `ProductSurfaceError` carries no free-text field at all, so the sanitized
/// constant is the whole of what crosses the membrane.
fn installed_extension_listing_error(
    error: ironclaw_extension_registry::ExtensionInstallationError,
) -> ProductSurfaceError {
    tracing::debug!(
        error = %error,
        "administrator-configuration view could not list installed extensions"
    );
    ProductSurfaceError::internal_from("installed-extension listing is unavailable")
}

fn map_admin_configuration_error(
    error: ironclaw_extension_host::AdminConfigurationServiceError,
) -> ProductSurfaceError {
    use ironclaw_extension_host::AdminConfigurationServiceError;
    let source = error.to_string();
    match error {
        AdminConfigurationServiceError::UnknownGroup => ProductSurfaceError::not_found(),
        AdminConfigurationServiceError::RevisionConflict { .. }
        | AdminConfigurationServiceError::IdempotencyConflict => service_error(
            ProductSurfaceErrorCode::Conflict,
            ProductSurfaceErrorKind::Conflict,
            409,
        ),
        AdminConfigurationServiceError::UnknownField
        | AdminConfigurationServiceError::DuplicateField
        | AdminConfigurationServiceError::MissingRequiredField
        | AdminConfigurationServiceError::ValueTooLarge => invalid_request(),
        AdminConfigurationServiceError::InvalidDescriptor
        | AdminConfigurationServiceError::DescriptorConflict => {
            tracing::error!(error = %source, "admin-configuration descriptor projection failed");
            ProductSurfaceError::internal_from("admin configuration descriptor is invalid")
        }
        AdminConfigurationServiceError::Unavailable => {
            tracing::warn!(error = %source, "admin-configuration query service unavailable");
            ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            }
        }
    }
}

fn invalid_request() -> ProductSurfaceError {
    service_error(
        ProductSurfaceErrorCode::InvalidRequest,
        ProductSurfaceErrorKind::Validation,
        400,
    )
}

fn forbidden() -> ProductSurfaceError {
    service_error(
        ProductSurfaceErrorCode::Forbidden,
        ProductSurfaceErrorKind::ParticipantDenied,
        403,
    )
}

fn service_error(
    code: ProductSurfaceErrorCode,
    kind: ProductSurfaceErrorKind,
    status_code: u16,
) -> ProductSurfaceError {
    ProductSurfaceError {
        code,
        kind,
        status_code,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_extension_registry::{
        ExtensionInstallation, ExtensionInstallationError, ExtensionInstallationId,
        ExtensionInstallationStorePort, ExtensionManifestRecord, MembershipDeactivation,
    };
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        ids::{ExtensionId, TenantId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };
    use ironclaw_secrets::{SecretStore, SecretStorePort};

    use super::*;

    fn caller(operator_config: bool) -> ProductSurfaceCaller {
        let mut caller = ProductSurfaceCaller::new(
            TenantId::new("tenant").expect("tenant id"),
            UserId::new("user").expect("user id"),
            None,
            None,
        );
        caller.operator_config = operator_config;
        caller
    }

    /// The fail-closed gate, checked *before* anything else.
    ///
    /// The ordering is the point, not just the 403. The provider's other early
    /// exit is a 503 for an unwired provider, and an unwired provider is
    /// exactly what this drives — the profile default that ships whenever
    /// administrator configuration is not deployed. So the test asserts the
    /// answer **differs by the flag on the same provider**: `false` gets 403,
    /// `true` gets 503. Asserting only the 403 would pass just as happily
    /// against a provider that refused every caller, and asserting it against
    /// a wired provider would not exercise the ordering at all. If the wiring
    /// check ran first, both callers would see 503 and an ungranted caller
    /// would learn whether the service is deployed.
    #[tokio::test]
    async fn the_operator_config_grant_is_checked_before_the_wiring_is_consulted() {
        let provider = AdminConfigurationViewProvider::default();

        let denied = provider
            .query(caller(false), serde_json::json!({}), None)
            .await
            .expect_err("a caller without operator_config must be refused");
        assert_eq!(denied.code, ProductSurfaceErrorCode::Forbidden);
        assert_eq!(denied.kind, ProductSurfaceErrorKind::ParticipantDenied);
        assert_eq!(denied.status_code, 403);
        assert!(
            !denied.retryable,
            "a denied caller must not be told to retry"
        );

        let granted = provider
            .query(caller(true), serde_json::json!({}), None)
            .await
            .expect_err("the same provider is unwired, so a granted caller gets 503");
        assert_eq!(
            granted.status_code, 503,
            "the same provider must answer differently for a granted caller, or the 403 \
             above proves nothing about the grant"
        );
        assert_ne!(
            denied.status_code, granted.status_code,
            "the grant, not the wiring, decides the 403"
        );
    }

    /// The view takes no parameters and is not paginated. Accepting either
    /// silently would let a caller believe a filter or a page was applied.
    #[tokio::test]
    async fn unsupported_params_or_a_cursor_are_rejected_rather_than_ignored() {
        let provider = AdminConfigurationViewProvider::default();
        for (params, cursor) in [
            (serde_json::json!({ "group_id": "smtp" }), None),
            (serde_json::json!({}), Some("page-2".to_string())),
        ] {
            let Err(error) = provider
                .query(caller(true), params.clone(), cursor.clone())
                .await
            else {
                panic!("unsupported query shape must be refused: params={params} cursor={cursor:?}")
            };
            assert_eq!(error.status_code, 400, "params={params} cursor={cursor:?}");
            assert_eq!(error.kind, ProductSurfaceErrorKind::Validation);
        }
    }

    /// An unwired provider is the profile default, so this arm runs in
    /// production whenever administrator configuration is not deployed. It
    /// must answer "unavailable, retryable" rather than an internal error —
    /// the capability may appear once the service is wired.
    #[tokio::test]
    async fn an_unwired_provider_reports_unavailable_rather_than_failing_internally() {
        let provider = AdminConfigurationViewProvider::default();
        let error = provider
            .query(caller(true), serde_json::json!({}), None)
            .await
            .expect_err("an unwired provider has nothing to list");
        assert_eq!(error.code, ProductSurfaceErrorCode::Unavailable);
        assert_eq!(error.status_code, 503);
    }

    #[test]
    fn the_provider_answers_for_the_administrator_configuration_view() {
        assert_eq!(
            AdminConfigurationViewProvider::default().descriptor().id,
            ADMIN_CONFIGURATION_VIEW.id,
            "the view id is the routing key; changing it silently reroutes the WebUI"
        );
    }

    /// Every service-error arm, with the answer each gives.
    ///
    /// Three distinctions are load-bearing and would be invisible if this were
    /// spot-checked: a revision/idempotency conflict is a 409 the caller can
    /// resolve by re-reading, a malformed *descriptor* is an internal 500
    /// (nothing the caller did), and `Unavailable` is the only retryable one.
    #[test]
    fn the_service_error_table_gives_each_failure_its_own_answer() {
        use ironclaw_extension_host::AdminConfigurationServiceError as E;

        use ProductSurfaceErrorCode as Code;
        use ProductSurfaceErrorKind as Kind;

        let cases = [
            (E::UnknownGroup, 404, false, Code::NotFound, Kind::NotFound),
            (
                E::RevisionConflict {
                    expected: 1,
                    actual: 2,
                },
                409,
                false,
                Code::Conflict,
                Kind::Conflict,
            ),
            (
                E::IdempotencyConflict,
                409,
                false,
                Code::Conflict,
                Kind::Conflict,
            ),
            (
                E::UnknownField,
                400,
                false,
                Code::InvalidRequest,
                Kind::Validation,
            ),
            (
                E::DuplicateField,
                400,
                false,
                Code::InvalidRequest,
                Kind::Validation,
            ),
            (
                E::MissingRequiredField,
                400,
                false,
                Code::InvalidRequest,
                Kind::Validation,
            ),
            (
                E::ValueTooLarge,
                400,
                false,
                Code::InvalidRequest,
                Kind::Validation,
            ),
            (
                E::InvalidDescriptor,
                500,
                false,
                Code::Internal,
                Kind::Internal,
            ),
            (
                E::DescriptorConflict,
                500,
                false,
                Code::Internal,
                Kind::Internal,
            ),
            (
                E::Unavailable,
                503,
                true,
                Code::Unavailable,
                Kind::ServiceUnavailable,
            ),
        ];

        for (error, status, retryable, code, kind) in cases {
            let label = error.to_string();
            let projected = map_admin_configuration_error(error);
            assert_eq!(projected.status_code, status, "status for {label}");
            assert_eq!(projected.retryable, retryable, "retryable for {label}");
            // Arms sharing a status must still project the right code/kind
            // pair — the WebUI branches on these, not on the raw number.
            assert_eq!(projected.code, code, "code for {label}");
            assert_eq!(projected.kind, kind, "kind for {label}");
        }
    }

    /// A secret field's value never reaches the view payload, even if the
    /// service hands one over.
    ///
    /// The service already redacts (`AdminConfigurationGroupState` is
    /// documented as redacted query state), so this guard is defense in depth
    /// — the same second lock `render_state` holds on the capability path. The
    /// sentinel drives the guard directly: if `render_group` ever goes back to
    /// passing `field.value` through unconditionally, this fails.
    #[test]
    fn a_secret_value_from_the_service_is_redacted_by_the_view() {
        use ironclaw_extension_host::AdminConfigurationFieldState;
        use ironclaw_extension_registry::AdminConfigurationGroupId;
        use ironclaw_host_api::ids::SecretHandle;

        let state = AdminConfigurationGroupState {
            group_id: AdminConfigurationGroupId::new("extension.fixture").expect("group id"),
            display_name: "Fixture".to_string(),
            description: String::new(),
            revision: 1,
            complete: true,
            fields: vec![
                AdminConfigurationFieldState {
                    handle: SecretHandle::new("api_token").expect("handle"),
                    label: "API token".to_string(),
                    secret: true,
                    required: true,
                    provided: true,
                    value: Some("sentinel-secret".to_string()),
                },
                AdminConfigurationFieldState {
                    handle: SecretHandle::new("region").expect("handle"),
                    label: "Region".to_string(),
                    secret: false,
                    required: false,
                    provided: true,
                    value: Some("eu-west-1".to_string()),
                },
            ],
        };

        let group = render_group(state, &[], &BTreeSet::new());
        assert_eq!(
            group.fields[0].value, None,
            "a secret value must not survive into the view payload"
        );
        assert_eq!(
            group.fields[1].value.as_deref(),
            Some("eu-west-1"),
            "a non-secret value must survive — redaction may not blank the whole form"
        );
    }

    /// The `installed` flag on each `used_by` entry is the whole reason this
    /// view reads the installation store at all: it is what tells the operator
    /// "this configuration group is used by an extension you have installed"
    /// versus "…by one you could install". Every existing case drives an empty
    /// installation list, which cannot tell a correct projection from one that
    /// hardcodes `false`.
    ///
    /// So this asserts the answer **differs by which extension is installed**,
    /// on one query against one group: the consumer whose `package_id` matches
    /// a listed installation reports `installed: true`, and its sibling in the
    /// same group -- listed by the catalog, absent from the store -- reports
    /// `false`. A projection that ignored the store, or that matched on the
    /// wrong field, fails one half or the other.
    #[tokio::test]
    async fn a_used_by_entry_is_marked_installed_only_when_the_store_lists_it() {
        use ironclaw_extension_registry::{
            AdminConfigurationField, AdminConfigurationGroupId,
            ExtensionAdminConfigurationDescriptor, ExtensionManifestRef, InstallationOwner,
        };
        use ironclaw_host_api::ids::SecretHandle;

        let installed = ExtensionId::new("installed-ext").expect("extension id");
        let group_id = AdminConfigurationGroupId::new("extension.fixture").expect("group id");
        let descriptor = ExtensionAdminConfigurationDescriptor {
            group_id: group_id.clone(),
            display_name: "Fixture".to_string(),
            description: String::new(),
            fields: vec![AdminConfigurationField {
                handle: SecretHandle::new("api_token").expect("handle"),
                label: "API token".to_string(),
                secret: true,
                required: true,
                host_managed: false,
            }],
        };
        let uses = vec![
            AdminConfigurationCatalogUse {
                descriptor: descriptor.clone(),
                package_id: installed.as_str().to_string(),
                display_name: "Installed Extension".to_string(),
            },
            AdminConfigurationCatalogUse {
                descriptor: descriptor.clone(),
                package_id: "absent-ext".to_string(),
                display_name: "Absent Extension".to_string(),
            },
        ];
        let installation = ExtensionInstallation::new(
            ExtensionInstallationId::new("installed-ext").expect("installation id"),
            installed.clone(),
            ExtensionManifestRef::new(installed, None),
            Vec::new(),
            chrono::Utc::now(),
            InstallationOwner::Tenant,
        )
        .expect("installation");

        let provider = AdminConfigurationViewProvider::new(
            Arc::new(admin_configuration_service(vec![descriptor])),
            uses,
            Arc::new(StaticInstallationStore(vec![installation])),
        );
        let page = provider
            .query(caller(true), serde_json::json!({}), None)
            .await
            .expect("a granted caller reads the administrator-configuration view");
        let response: RebornAdminConfigurationListResponse =
            serde_json::from_value(page.payload).expect("list response");

        let group = response
            .groups
            .iter()
            .find(|group| group.group_id == group_id.as_str())
            .expect("the registered descriptor renders one group");
        let flags: Vec<(&str, bool)> = group
            .used_by
            .iter()
            .map(|usage| (usage.package_id.as_str(), usage.installed))
            .collect();
        assert_eq!(
            flags,
            vec![("installed-ext", true), ("absent-ext", false)],
            "only the consumer the installation store lists may report installed"
        );
    }

    /// The wiring `provider_with_unavailable_installation_store` builds, with
    /// the descriptor set left to the caller so a case can register a group
    /// for the view to render.
    fn admin_configuration_service(
        descriptors: Vec<ironclaw_extension_registry::ExtensionAdminConfigurationDescriptor>,
    ) -> ComposedAdminConfigurationService {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let secrets: Arc<dyn SecretStorePort> = Arc::new(SecretStore::ephemeral());
        ComposedAdminConfigurationService::new(
            ironclaw_extension_host::FilesystemAdminConfigurationStore::new(Arc::new(
                ScopedFilesystem::new(filesystem, |_scope| {
                    MountView::new(vec![MountGrant::new(
                        MountAlias::new("/extension-admin-configuration").expect("mount alias"),
                        VirtualPath::new("/tenants/test/shared/admin-configuration")
                            .expect("virtual path"),
                        MountPermissions::read_write_list_delete(),
                    )])
                }),
            )),
            secrets,
            descriptors,
        )
        .expect("admin configuration service")
    }

    /// An installation store that lists a fixed set and refuses everything
    /// else, so a case can control exactly which extensions read as installed.
    struct StaticInstallationStore(Vec<ExtensionInstallation>);

    #[async_trait]
    impl ExtensionInstallationStorePort for StaticInstallationStore {
        async fn list_manifests(
            &self,
        ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
            Ok(Vec::new())
        }

        async fn get_manifest(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
            Ok(None)
        }

        async fn persist_removal_tombstone(
            &self,
            _manifest: ExtensionManifestRecord,
        ) -> Result<(), ExtensionInstallationError> {
            Ok(())
        }

        async fn upsert_manifest_and_installation(
            &self,
            _manifest: ExtensionManifestRecord,
            _installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            Ok(())
        }

        async fn list_installations(
            &self,
        ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
            Ok(self.0.clone())
        }

        async fn get_installation(
            &self,
            installation_id: &ExtensionInstallationId,
        ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
            Ok(self
                .0
                .iter()
                .find(|installation| installation.installation_id() == installation_id)
                .cloned())
        }

        async fn upsert_installation(
            &self,
            _installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            Ok(())
        }

        async fn activate_membership(
            &self,
            installation_id: &ExtensionInstallationId,
            _user_id: &UserId,
        ) -> Result<ExtensionInstallation, ExtensionInstallationError> {
            Err(ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            })
        }

        async fn deactivate_membership(
            &self,
            installation_id: &ExtensionInstallationId,
            _user_id: &UserId,
        ) -> Result<MembershipDeactivation, ExtensionInstallationError> {
            Err(ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            })
        }

        async fn delete_installation(
            &self,
            _installation_id: &ExtensionInstallationId,
        ) -> Result<(), ExtensionInstallationError> {
            Ok(())
        }

        async fn delete_manifest(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<(), ExtensionInstallationError> {
            Ok(())
        }
    }

    /// An installation store that cannot serve a read, with the backend text
    /// carrying the shape the reviewer flagged: a host mount path.
    struct UnavailableInstallationStore {
        reason: &'static str,
    }

    impl UnavailableInstallationStore {
        fn error(&self) -> ExtensionInstallationError {
            ExtensionInstallationError::StoreUnavailable {
                reason: self.reason.to_string(),
            }
        }
    }

    #[async_trait]
    impl ExtensionInstallationStorePort for UnavailableInstallationStore {
        async fn list_manifests(
            &self,
        ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
            Err(self.error())
        }

        async fn get_manifest(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
            Err(self.error())
        }

        async fn persist_removal_tombstone(
            &self,
            _manifest: ExtensionManifestRecord,
        ) -> Result<(), ExtensionInstallationError> {
            Err(self.error())
        }

        async fn upsert_manifest_and_installation(
            &self,
            _manifest: ExtensionManifestRecord,
            _installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            Err(self.error())
        }

        async fn list_installations(
            &self,
        ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
            Err(self.error())
        }

        async fn get_installation(
            &self,
            _installation_id: &ExtensionInstallationId,
        ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
            Err(self.error())
        }

        async fn upsert_installation(
            &self,
            _installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            Err(self.error())
        }

        async fn activate_membership(
            &self,
            _installation_id: &ExtensionInstallationId,
            _user_id: &UserId,
        ) -> Result<ExtensionInstallation, ExtensionInstallationError> {
            Err(self.error())
        }

        async fn deactivate_membership(
            &self,
            _installation_id: &ExtensionInstallationId,
            _user_id: &UserId,
        ) -> Result<MembershipDeactivation, ExtensionInstallationError> {
            Err(self.error())
        }

        async fn delete_installation(
            &self,
            _installation_id: &ExtensionInstallationId,
        ) -> Result<(), ExtensionInstallationError> {
            Err(self.error())
        }

        async fn delete_manifest(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<(), ExtensionInstallationError> {
            Err(self.error())
        }
    }

    /// A wired provider whose administrator-configuration service resolves
    /// (no declared groups, so `list` succeeds trivially) but whose
    /// installation store cannot answer.
    ///
    /// Empty descriptors are deliberate: they isolate this test on the one
    /// call the reviewer flagged, and they are the harder case — a store read
    /// that was silently absorbed would still produce a perfectly valid empty
    /// page here, which is the regression this test has to be able to see.
    fn provider_with_unavailable_installation_store(
        reason: &'static str,
    ) -> AdminConfigurationViewProvider {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let secrets: Arc<dyn SecretStorePort> = Arc::new(SecretStore::ephemeral());
        let service = ComposedAdminConfigurationService::new(
            ironclaw_extension_host::FilesystemAdminConfigurationStore::new(Arc::new(
                ScopedFilesystem::new(filesystem, |_scope| {
                    MountView::new(vec![MountGrant::new(
                        MountAlias::new("/extension-admin-configuration").expect("mount alias"),
                        VirtualPath::new("/tenants/test/shared/admin-configuration")
                            .expect("virtual path"),
                        MountPermissions::read_write_list_delete(),
                    )])
                }),
            )),
            secrets,
            Vec::new(),
        )
        .expect("admin configuration service");
        AdminConfigurationViewProvider::new(
            Arc::new(service),
            Vec::new(),
            Arc::new(UnavailableInstallationStore { reason }),
        )
    }

    #[derive(Clone, Default)]
    struct SharedLogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    struct SharedLogWriterGuard(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedLogWriterGuard {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogWriter {
        type Writer = SharedLogWriterGuard;

        fn make_writer(&'a self) -> Self::Writer {
            SharedLogWriterGuard(std::sync::Arc::clone(&self.0))
        }
    }

    impl SharedLogWriter {
        fn contents(&self) -> String {
            String::from_utf8(
                self.0
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone(),
            )
            .expect("tracing output is UTF-8")
        }
    }

    /// A failing installation store is surfaced, sanitized, and still
    /// diagnosable — the three halves that have to hold together.
    ///
    /// `ExtensionInstallationStorePort` is filesystem-backed, so its `Display`
    /// can name a host mount path; the sentinel below is that shape. The
    /// assertions are:
    ///
    /// - the failure is **not absorbed** — the caller gets an error, not a
    ///   page whose `used_by` entries silently all read `installed: false`;
    /// - the caller's error is **sanitized** — nothing about the backend
    ///   crosses the membrane, checked on the serialized wire form rather
    ///   than on the struct, so adding a free-text field later would fail
    ///   here instead of passing vacuously;
    /// - the cause is **not discarded**, and lands at `debug!` *only*. This is
    ///   the half that moved: `ProductSurfaceError::internal_from` logs
    ///   whatever it is handed at `error!`, so passing the store's own text to
    ///   it put a mount path on the always-on line. Asserting "the cause is
    ///   logged somewhere" would pass just as happily against that, so the
    ///   test reads the emitted level per line and requires every line naming
    ///   the sentinel to be a `DEBUG` one.
    ///
    /// A subscriber has to be installed for the last two to mean anything:
    /// `tracing` short-circuits on the null dispatcher, so without one the
    /// macro body never runs and the test could not tell "recorded it" from
    /// "dropped it". Scoped with `with_default`, so parallel tests are
    /// unaffected.
    ///
    /// Driven on an explicit current-thread runtime rather than `#[tokio::test]`
    /// because `with_default` scopes the subscriber to a *synchronous* closure;
    /// the whole query has to complete inside it.
    #[test]
    fn a_failing_installation_store_is_sanitized_for_the_caller_and_still_diagnosable() {
        let sentinel = "/var/lib/ironclaw/mounts/tenant-alpha/extensions.db is unreadable";
        let provider = provider_with_unavailable_installation_store(sentinel);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let logs = SharedLogWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_target(false)
            // The level prefix is read below, so it must not be wrapped in
            // ANSI colour escapes.
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(logs.clone())
            .finish();

        let error = tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(provider.query(caller(true), serde_json::json!({}), None))
        })
        .expect_err("a store that cannot list installations must fail the view, not empty it");

        assert_eq!(error.code, ProductSurfaceErrorCode::Internal);
        assert_eq!(error.status_code, 500);
        assert!(!error.retryable);
        assert_eq!(error.field, None);
        assert_eq!(error.validation_code, None);
        let on_the_wire = serde_json::to_string(&error).expect("surface error serializes");
        assert!(
            !on_the_wire.contains("/var/lib/ironclaw"),
            "the store's mount path must not cross the membrane: {on_the_wire}"
        );

        let logged = logs.contents();
        let carrying_the_cause = logged
            .lines()
            .filter(|line| line.contains(sentinel))
            .collect::<Vec<_>>();
        assert!(
            !carrying_the_cause.is_empty(),
            "the store's cause must survive in the log, got {logged:?}"
        );
        assert!(
            carrying_the_cause
                .iter()
                .all(|line| line.trim_start().starts_with("DEBUG")),
            "the store's own text belongs to the debug diagnostic only — an always-on line \
             carrying it puts a mount path in every deployment's log: {carrying_the_cause:?}"
        );
        assert!(
            logged.contains("could not list installed extensions"),
            "the diagnostic keeps its stable message, got {logged:?}"
        );
    }

    /// No arm of the table hands the caller a field name or a validation code.
    ///
    /// `ProductSurfaceError` carries `field` and `validation_code` precisely so
    /// a *validated form submission* can point at the offending input. This is
    /// a read-only list view: there is no submitted field to blame, and
    /// `AdminConfigurationServiceError`'s own `Display` names internals
    /// (`RevisionConflict { expected, actual }`) that a caller has no business
    /// seeing. So every arm must leave both slots empty — including the two
    /// that log the source before answering.
    #[test]
    fn no_arm_of_the_table_hands_the_caller_a_field_or_a_validation_code() {
        use ironclaw_extension_host::AdminConfigurationServiceError as E;

        for error in [
            E::UnknownGroup,
            E::RevisionConflict {
                expected: 1,
                actual: 2,
            },
            E::IdempotencyConflict,
            E::UnknownField,
            E::DuplicateField,
            E::MissingRequiredField,
            E::ValueTooLarge,
            E::InvalidDescriptor,
            E::DescriptorConflict,
            E::Unavailable,
        ] {
            let label = error.to_string();
            let projected = map_admin_configuration_error(error);
            assert!(
                projected.field.is_none(),
                "{label} leaked a field name to the caller"
            );
            assert!(
                projected.validation_code.is_none(),
                "{label} leaked a validation code to the caller"
            );
        }
    }
}
