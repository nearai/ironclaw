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
    RebornProviderAdmin, RebornProviderAdminError, RebornProviderSelection, RebornProviderStatus,
    RebornProviderWriteOutcome,
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
    routes: &'static str,
    default: Option<ProductSafeProviderSelection>,
    v1_state: &'static str,
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
    v1_state: &'static str,
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
        RebornProviderAdminError::UnknownProvider {
            provider, known, ..
        } => ProductWorkflowError::InvalidBindingRequest {
            reason: format!(
                "unknown Reborn LLM provider `{}`; available providers: {}",
                provider,
                known.join(", ")
            ),
        },
        RebornProviderAdminError::InvalidRequest { reason } => {
            ProductWorkflowError::InvalidBindingRequest { reason }
        }
        RebornProviderAdminError::LoadRegistry { .. }
        | RebornProviderAdminError::LoadConfig { .. }
        | RebornProviderAdminError::UpdateConfig { .. } => ProductWorkflowError::Transient {
            reason: "provider-admin operation failed".to_string(),
        },
    }
}
