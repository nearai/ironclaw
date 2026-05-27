use async_trait::async_trait;
use ironclaw_product_adapters::{
    ProductCommandResultPayload, ProductInboundAck, ProductRejection, ProductRejectionKind,
};
use ironclaw_product_workflow::{
    ProductCommand, ProductCommandContext, ProductCommandService, ProductModelCommand,
    ProductWorkflowError,
};

use crate::{RebornProviderAdmin, RebornProviderAdminError};

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

        let payload = match action {
            ProductModelCommand::Status => {
                serde_json::to_value(self.admin.status().map_err(provider_admin_workflow_error)?)
            }
            ProductModelCommand::Set { model } => serde_json::to_value(
                self.admin
                    .set_model(&model)
                    .map_err(provider_admin_workflow_error)?,
            ),
            ProductModelCommand::SetProvider { provider, model } => serde_json::to_value(
                self.admin
                    .set_provider(&provider, model.as_deref())
                    .map_err(provider_admin_workflow_error)?,
            ),
        }
        .map_err(|error| ProductWorkflowError::Transient {
            reason: format!("provider-admin response serialization failed: {error}"),
        })?;

        Ok(ProductInboundAck::CommandResult {
            command: "model".to_string(),
            payload: ProductCommandResultPayload::new(payload),
        })
    }
}

fn provider_admin_workflow_error(error: RebornProviderAdminError) -> ProductWorkflowError {
    if error.is_invalid_request() {
        ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        }
    } else {
        ProductWorkflowError::Transient {
            reason: error.to_string(),
        }
    }
}
