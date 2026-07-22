//! Descriptor-backed operator configuration read projections.

use futures::future::try_join_all;
use serde::Deserialize;

use super::{
    AUTO_APPROVE_CONFIG_KEY, ProductCapabilityInvoker, RebornOperatorConfigGetResponse,
    RebornOperatorConfigListResponse, RebornOperatorConfigValidateRequest,
    RebornOperatorConfigValidateResponse, RebornServices, RebornServicesError,
    RebornViewDescriptor, RebornViewProvider, TOOL_CONFIG_PREFIX, WebUiAuthenticatedCaller,
    auto_approve_config_entry, caller_resource_scope, find_operator_tool,
    operator_config_not_wired_response, operator_config_unknown_key_error,
    operator_config_validation_diagnostics, operator_tool_permission_context, tool_config_entry,
    tool_config_entry_with_context,
};

pub const OPERATOR_CONFIG_LIST_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "operator_config_list",
    paginated: false,
};

pub const OPERATOR_CONFIG_KEY_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "operator_config_key",
    paginated: false,
};

pub const OPERATOR_CONFIG_VALIDATE_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "operator_config_validate",
    paginated: false,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OperatorConfigKeyViewParams {
    key: String,
}

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    pub(super) async fn build_operator_config_list_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorConfigListResponse, RebornServicesError> {
        let Some(config) = &self.operator_approval_config else {
            return Ok(operator_config_not_wired_response());
        };
        let scope = caller_resource_scope(&caller);
        let mut entries = vec![auto_approve_config_entry(config, &scope).await?];
        let tools = config
            .tool_catalog
            .list_operator_tools(&scope.user_id)
            .await;
        let tool_context = operator_tool_permission_context(config, &scope, &tools).await?;
        entries.extend(
            try_join_all(
                tools
                    .iter()
                    .map(|tool| tool_config_entry_with_context(&tool_context, tool)),
            )
            .await?,
        );
        Ok(RebornOperatorConfigListResponse {
            entries,
            precedence: vec![
                "locked".to_string(),
                "override".to_string(),
                "global".to_string(),
                "default".to_string(),
            ],
            diagnostics: Vec::new(),
        })
    }

    pub(super) async fn build_operator_config_key_view(
        &self,
        caller: WebUiAuthenticatedCaller,
        params: serde_json::Value,
    ) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
        let OperatorConfigKeyViewParams { key } =
            serde_json::from_value(params).map_err(RebornServicesError::internal_from)?;
        let Some(config) = &self.operator_approval_config else {
            let _ = (caller, key);
            return Err(RebornServicesError::service_unavailable(false));
        };
        let scope = caller_resource_scope(&caller);
        let entry = if key == AUTO_APPROVE_CONFIG_KEY {
            auto_approve_config_entry(config, &scope).await?
        } else if let Some(capability_id) = key.strip_prefix(TOOL_CONFIG_PREFIX) {
            let tool = find_operator_tool(config, capability_id, &scope.user_id).await?;
            tool_config_entry(config, &scope, &tool).await?
        } else {
            return Err(operator_config_unknown_key_error("key"));
        };
        Ok(RebornOperatorConfigGetResponse { entry })
    }

    pub(super) fn build_operator_config_validate_view(
        &self,
        params: serde_json::Value,
    ) -> Result<RebornOperatorConfigValidateResponse, RebornServicesError> {
        let request: RebornOperatorConfigValidateRequest =
            serde_json::from_value(params).map_err(RebornServicesError::internal_from)?;
        let diagnostics = operator_config_validation_diagnostics(request.keys);
        Ok(RebornOperatorConfigValidateResponse {
            valid: diagnostics.is_empty(),
            diagnostics,
        })
    }
}
