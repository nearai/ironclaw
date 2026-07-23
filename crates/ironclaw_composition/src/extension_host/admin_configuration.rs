//! Manifest-driven tenant administrator configuration adapters.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_host::{AdminConfigurationGroupState, AdminConfigurationService};
use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{InvocationId, ResourceScope};
use ironclaw_product_workflow::{
    ADMIN_CONFIGURATION_VIEW, IronClawAdminConfigurationField, IronClawAdminConfigurationGroup,
    IronClawAdminConfigurationListResponse, IronClawAdminConfigurationUse, IronClawServicesError,
    IronClawServicesErrorCode, IronClawServicesErrorKind, IronClawViewDescriptor, IronClawViewPage,
    IronClawViewProvider, WebUiAuthenticatedCaller,
};
use ironclaw_secrets::SecretStore;

use crate::extension_host::available_extensions::AdminConfigurationCatalogUse;

pub(crate) type ComposedAdminConfigurationService =
    AdminConfigurationService<dyn RootFilesystem, dyn SecretStore>;

#[derive(Clone, Default)]
pub(crate) struct AdminConfigurationViewProvider {
    parts: Option<Arc<AdminConfigurationViewParts>>,
}

struct AdminConfigurationViewParts {
    service: Arc<ComposedAdminConfigurationService>,
    uses: Arc<Vec<AdminConfigurationCatalogUse>>,
    installation_store: Arc<dyn ExtensionInstallationStore>,
}

impl AdminConfigurationViewProvider {
    pub(crate) fn new(
        service: Arc<ComposedAdminConfigurationService>,
        uses: Vec<AdminConfigurationCatalogUse>,
        installation_store: Arc<dyn ExtensionInstallationStore>,
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
impl IronClawViewProvider for AdminConfigurationViewProvider {
    fn descriptor(&self) -> IronClawViewDescriptor {
        ADMIN_CONFIGURATION_VIEW
    }

    async fn query(
        &self,
        caller: WebUiAuthenticatedCaller,
        params: serde_json::Value,
        cursor: Option<String>,
    ) -> Result<IronClawViewPage, IronClawServicesError> {
        if !caller.operator_webui_config {
            return Err(forbidden());
        }
        if params != serde_json::json!({}) || cursor.is_some() {
            return Err(invalid_request());
        }
        let Some(parts) = &self.parts else {
            return Err(service_error(
                IronClawServicesErrorCode::Unavailable,
                IronClawServicesErrorKind::ServiceUnavailable,
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
            .map_err(|error| IronClawServicesError::internal_from(error.to_string()))?
            .into_iter()
            .map(|installation| installation.extension_id().as_str().to_string())
            .collect::<BTreeSet<_>>();
        let groups = states
            .into_iter()
            .map(|state| render_group(state, &parts.uses, &installed))
            .collect();
        let payload = serde_json::to_value(IronClawAdminConfigurationListResponse { groups })
            .map_err(IronClawServicesError::internal_from)?;
        Ok(IronClawViewPage {
            payload,
            next_cursor: None,
        })
    }
}

fn render_group(
    state: AdminConfigurationGroupState,
    uses: &[AdminConfigurationCatalogUse],
    installed: &BTreeSet<String>,
) -> IronClawAdminConfigurationGroup {
    let group_id = state.group_id.as_str().to_string();
    IronClawAdminConfigurationGroup {
        used_by: uses
            .iter()
            .filter(|usage| usage.descriptor.group_id == state.group_id)
            .map(|usage| IronClawAdminConfigurationUse {
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
            .map(|field| IronClawAdminConfigurationField {
                handle: field.handle.as_str().to_string(),
                label: field.label,
                secret: field.secret,
                required: field.required,
                provided: field.provided,
                value: field.value,
            })
            .collect(),
    }
}

pub(crate) fn caller_scope(caller: &WebUiAuthenticatedCaller) -> ResourceScope {
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

fn map_admin_configuration_error(
    error: ironclaw_extension_host::AdminConfigurationServiceError,
) -> IronClawServicesError {
    use ironclaw_extension_host::AdminConfigurationServiceError;
    let source = error.to_string();
    match error {
        AdminConfigurationServiceError::UnknownGroup => IronClawServicesError::not_found(),
        AdminConfigurationServiceError::RevisionConflict { .. }
        | AdminConfigurationServiceError::IdempotencyConflict => service_error(
            IronClawServicesErrorCode::Conflict,
            IronClawServicesErrorKind::Conflict,
            409,
        ),
        AdminConfigurationServiceError::UnknownField
        | AdminConfigurationServiceError::DuplicateField
        | AdminConfigurationServiceError::MissingRequiredField
        | AdminConfigurationServiceError::ValueTooLarge => invalid_request(),
        AdminConfigurationServiceError::InvalidDescriptor
        | AdminConfigurationServiceError::DescriptorConflict => {
            tracing::error!(error = %source, "admin-configuration descriptor projection failed");
            IronClawServicesError::internal_from("admin configuration descriptor is invalid")
        }
        AdminConfigurationServiceError::Unavailable => {
            tracing::warn!(error = %source, "admin-configuration query service unavailable");
            IronClawServicesError {
                code: IronClawServicesErrorCode::Unavailable,
                kind: IronClawServicesErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            }
        }
    }
}

fn invalid_request() -> IronClawServicesError {
    service_error(
        IronClawServicesErrorCode::InvalidRequest,
        IronClawServicesErrorKind::Validation,
        400,
    )
}

fn forbidden() -> IronClawServicesError {
    service_error(
        IronClawServicesErrorCode::Forbidden,
        IronClawServicesErrorKind::ParticipantDenied,
        403,
    )
}

fn service_error(
    code: IronClawServicesErrorCode,
    kind: IronClawServicesErrorKind,
    status_code: u16,
) -> IronClawServicesError {
    IronClawServicesError {
        code,
        kind,
        status_code,
        retryable: false,
        field: None,
        validation_code: None,
    }
}
