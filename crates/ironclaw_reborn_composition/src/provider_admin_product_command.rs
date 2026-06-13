use async_trait::async_trait;
use ironclaw_product_adapters::{
    ProductCommandResultPayload, ProductInboundAck, ProductRejection, ProductRejectionKind,
};
use ironclaw_product_workflow::{
    ProductCommand, ProductCommandContext, ProductCommandService, ProductModelCommand,
    ProductWorkflowError,
};
use serde::Serialize;

use crate::{
    RebornModelRoutesState, RebornProviderAdmin, RebornProviderAdminError, RebornProviderSelection,
    RebornProviderStatus, RebornProviderWriteOutcome, RebornV1State,
};

pub struct RebornProviderAdminProductCommandService {
    admin: RebornProviderAdmin,
}

impl RebornProviderAdminProductCommandService {
    pub fn new(admin: RebornProviderAdmin) -> Self {
        Self { admin }
    }
}

#[async_trait]
impl ProductCommandService for RebornProviderAdminProductCommandService {
    async fn execute(
        &self,
        _context: ProductCommandContext,
        command: ProductCommand,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let ProductCommand::Model { action } = command else {
            return Ok(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("command routing unavailable: {}", command.name()),
            )));
        };

        let admin = self.admin.clone();
        let payload = tokio::task::spawn_blocking(move || provider_admin_payload(admin, action))
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("provider-admin task failed: {error}"),
            })??;

        Ok(ProductInboundAck::CommandResult {
            command: "model".to_string(),
            payload: ProductCommandResultPayload::new(payload),
        })
    }
}

fn provider_admin_payload(
    admin: RebornProviderAdmin,
    action: ProductModelCommand,
) -> Result<serde_json::Value, ProductWorkflowError> {
    let payload = match action {
        ProductModelCommand::Status => {
            ProductSafeProviderStatus::from(admin.status().map_err(provider_admin_workflow_error)?)
                .to_value()
        }
        ProductModelCommand::Set { model } => ProductSafeProviderWriteOutcome::from(
            admin
                .set_model(&model)
                .map_err(provider_admin_workflow_error)?,
        )
        .to_value(),
        ProductModelCommand::SetProvider { provider, model } => {
            ProductSafeProviderWriteOutcome::from(
                admin
                    .set_provider(&provider, model.as_deref())
                    .map_err(provider_admin_workflow_error)?,
            )
            .to_value()
        }
    };
    payload.map_err(|error| ProductWorkflowError::Transient {
        reason: format!("provider-admin response serialization failed: {error}"),
    })
}

#[derive(Serialize)]
struct ProductSafeProviderStatus {
    routes: RebornModelRoutesState,
    default: Option<ProductSafeProviderSelection>,
    v1_state: RebornV1State,
}

impl From<RebornProviderStatus> for ProductSafeProviderStatus {
    fn from(status: RebornProviderStatus) -> Self {
        Self {
            routes: status.routes,
            default: status.default.map(ProductSafeProviderSelection::from),
            v1_state: status.v1_state,
        }
    }
}

impl ProductSafeProviderStatus {
    fn to_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

#[derive(Serialize)]
struct ProductSafeProviderSelection {
    provider_id: Option<String>,
    provider_known: bool,
    model: Option<String>,
}

impl From<RebornProviderSelection> for ProductSafeProviderSelection {
    fn from(selection: RebornProviderSelection) -> Self {
        Self {
            provider_id: selection.provider_id,
            provider_known: selection.provider_known,
            model: selection.model,
        }
    }
}

#[derive(Serialize)]
struct ProductSafeProviderWriteOutcome {
    provider_id: String,
    model: String,
    api_key_required: bool,
    missing_api_key: bool,
    v1_state: RebornV1State,
}

impl From<RebornProviderWriteOutcome> for ProductSafeProviderWriteOutcome {
    fn from(outcome: RebornProviderWriteOutcome) -> Self {
        Self {
            provider_id: outcome.provider_id,
            model: outcome.model,
            api_key_required: outcome.api_key_required,
            missing_api_key: outcome.missing_api_key,
            v1_state: outcome.v1_state,
        }
    }
}

impl ProductSafeProviderWriteOutcome {
    fn to_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

fn provider_admin_workflow_error(error: RebornProviderAdminError) -> ProductWorkflowError {
    match error {
        RebornProviderAdminError::UnknownProvider { provider, .. } => {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("unknown Reborn LLM provider `{provider}`"),
            }
        }
        RebornProviderAdminError::InvalidRequest { reason } => {
            ProductWorkflowError::InvalidBindingRequest { reason }
        }
        RebornProviderAdminError::LoadRegistry { reason, .. } => ProductWorkflowError::Transient {
            reason: format!("load Reborn provider catalog failed: {reason}"),
        },
        RebornProviderAdminError::LoadConfig { source, .. } => ProductWorkflowError::Transient {
            reason: format!(
                "load Reborn config failed: {}",
                config_load_error_reason(source.as_ref())
            ),
        },
        RebornProviderAdminError::UpdateConfig { source, .. } => ProductWorkflowError::Transient {
            reason: format!(
                "update Reborn config failed: {}",
                config_update_error_reason(source.as_ref())
            ),
        },
    }
}

fn config_load_error_reason(error: &ironclaw_reborn_config::RebornConfigFileError) -> String {
    match error {
        ironclaw_reborn_config::RebornConfigFileError::Io { source, .. } => {
            format!("read failed: {source}")
        }
        ironclaw_reborn_config::RebornConfigFileError::Toml { source, .. } => {
            format!("TOML parse failed: {source}")
        }
        ironclaw_reborn_config::RebornConfigFileError::IncompatibleApiVersion {
            found,
            expected,
            ..
        } => {
            format!("api_version `{found}` is incompatible with `{expected}`")
        }
        ironclaw_reborn_config::RebornConfigFileError::InlineSecret { source, .. } => {
            format!("field validation failed: {source}")
        }
        ironclaw_reborn_config::RebornConfigFileError::InvalidField { field, reason, .. } => {
            format!("field `{field}` validation failed: {reason}")
        }
        ironclaw_reborn_config::RebornConfigFileError::InvalidApiVersion {
            found, reason, ..
        } => {
            format!("api_version `{found}` could not be parsed: {reason}")
        }
    }
}

fn config_update_error_reason(
    error: &ironclaw_reborn_config::RebornConfigFileUpdateError,
) -> String {
    match error {
        ironclaw_reborn_config::RebornConfigFileUpdateError::Lock { source, .. } => {
            format!("lock failed: {source}")
        }
        ironclaw_reborn_config::RebornConfigFileUpdateError::Read { source, .. } => {
            format!("read failed: {source}")
        }
        ironclaw_reborn_config::RebornConfigFileUpdateError::Parse { source, .. } => {
            format!("TOML parse failed: {source}")
        }
        ironclaw_reborn_config::RebornConfigFileUpdateError::Validate { source, .. } => {
            format!("validation failed: {}", config_load_error_reason(source))
        }
        ironclaw_reborn_config::RebornConfigFileUpdateError::Write { source, .. } => {
            format!("write failed: {source}")
        }
    }
}
