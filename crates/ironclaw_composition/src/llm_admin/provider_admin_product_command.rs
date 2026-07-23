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
    IronClawModelRoutesState, IronClawProviderAdmin, IronClawProviderAdminError,
    IronClawProviderSelection, IronClawProviderStatus, IronClawProviderWriteOutcome,
    IronClawV1State,
};

pub struct IronClawProviderAdminProductCommandService {
    admin: IronClawProviderAdmin,
}

impl IronClawProviderAdminProductCommandService {
    pub fn new(admin: IronClawProviderAdmin) -> Self {
        Self { admin }
    }
}

#[async_trait]
impl ProductCommandService for IronClawProviderAdminProductCommandService {
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
    admin: IronClawProviderAdmin,
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
    routes: IronClawModelRoutesState,
    default: Option<ProductSafeProviderSelection>,
    v1_state: IronClawV1State,
}

impl From<IronClawProviderStatus> for ProductSafeProviderStatus {
    fn from(status: IronClawProviderStatus) -> Self {
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

impl From<IronClawProviderSelection> for ProductSafeProviderSelection {
    fn from(selection: IronClawProviderSelection) -> Self {
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
    v1_state: IronClawV1State,
}

impl From<IronClawProviderWriteOutcome> for ProductSafeProviderWriteOutcome {
    fn from(outcome: IronClawProviderWriteOutcome) -> Self {
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

fn provider_admin_workflow_error(error: IronClawProviderAdminError) -> ProductWorkflowError {
    match error {
        IronClawProviderAdminError::UnknownProvider { provider, .. } => {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("unknown IronClaw LLM provider `{provider}`"),
            }
        }
        IronClawProviderAdminError::InvalidRequest { reason } => {
            ProductWorkflowError::InvalidBindingRequest { reason }
        }
        IronClawProviderAdminError::LoadRegistry { reason, .. } => {
            ProductWorkflowError::Transient {
                reason: format!("load IronClaw provider catalog failed: {reason}"),
            }
        }
        IronClawProviderAdminError::LoadConfig { source, .. } => ProductWorkflowError::Transient {
            reason: format!(
                "load IronClaw config failed: {}",
                config_load_error_reason(source.as_ref())
            ),
        },
        IronClawProviderAdminError::UpdateConfig { source, .. } => {
            ProductWorkflowError::Transient {
                reason: format!(
                    "update IronClaw config failed: {}",
                    config_update_error_reason(source.as_ref())
                ),
            }
        }
        IronClawProviderAdminError::EnvDetection { source } => {
            tracing::debug!(
                error = %source,
                "environment LLM detection failed while handling a product LLM-admin command"
            );
            ProductWorkflowError::InvalidBindingRequest {
                reason: "environment provider detection failed; check provider environment \
                         variables"
                    .to_string(),
            }
        }
    }
}

fn config_load_error_reason(error: &ironclaw_config::IronClawConfigFileError) -> String {
    match error {
        ironclaw_config::IronClawConfigFileError::Io { source, .. } => {
            format!("read failed: {source}")
        }
        ironclaw_config::IronClawConfigFileError::Toml { source, .. } => {
            format!("TOML parse failed: {source}")
        }
        ironclaw_config::IronClawConfigFileError::IncompatibleApiVersion {
            found,
            expected,
            ..
        } => {
            format!("api_version `{found}` is incompatible with `{expected}`")
        }
        ironclaw_config::IronClawConfigFileError::InlineSecret { source, .. } => {
            format!("field validation failed: {source}")
        }
        ironclaw_config::IronClawConfigFileError::InvalidField { field, reason, .. } => {
            format!("field `{field}` validation failed: {reason}")
        }
        ironclaw_config::IronClawConfigFileError::InvalidApiVersion { found, reason, .. } => {
            format!("api_version `{found}` could not be parsed: {reason}")
        }
    }
}

fn config_update_error_reason(error: &ironclaw_config::IronClawConfigFileUpdateError) -> String {
    match error {
        ironclaw_config::IronClawConfigFileUpdateError::Lock { source, .. } => {
            format!("lock failed: {source}")
        }
        ironclaw_config::IronClawConfigFileUpdateError::Read { source, .. } => {
            format!("read failed: {source}")
        }
        ironclaw_config::IronClawConfigFileUpdateError::Parse { source, .. } => {
            format!("TOML parse failed: {source}")
        }
        ironclaw_config::IronClawConfigFileUpdateError::Validate { source, .. } => {
            format!("validation failed: {}", config_load_error_reason(source))
        }
        ironclaw_config::IronClawConfigFileUpdateError::Write { source, .. } => {
            format!("write failed: {source}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EnvDetection` denotes incomplete/invalid operator env configuration
    /// (`IronClawProviderAdmin::detect_env_llm`'s "partial env" outcome), not a
    /// transient backend failure — it must map to `InvalidBindingRequest` so
    /// callers don't retry a config problem as if it were flaky.
    #[test]
    fn env_detection_maps_to_invalid_binding_request_not_transient() {
        let error = IronClawProviderAdminError::EnvDetection {
            source: Box::new(ironclaw_llm::LlmError::InvalidResponse {
                provider: "openai".to_string(),
                reason: "OPENAI_API_KEY is unset but OPENAI_MODEL is set".to_string(),
            }),
        };
        let mapped = provider_admin_workflow_error(error);
        assert!(
            matches!(mapped, ProductWorkflowError::InvalidBindingRequest { .. }),
            "EnvDetection must map to InvalidBindingRequest, not Transient: {mapped:?}"
        );
    }
}
