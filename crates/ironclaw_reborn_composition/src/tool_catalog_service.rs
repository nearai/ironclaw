//! Composition-side implementation of the WebUI tool catalog port.
//!
//! Enumerates the installed capabilities from the extension registry so the
//! WebUI tools tab can list every tool and show its resolved per-tool
//! permission. The product facade depends only on the
//! [`ToolCatalogService`] port; this impl is the single place that reaches the
//! capability registry.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::ToolPermissionState;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::PermissionMode;
use ironclaw_product_workflow::{
    RebornServicesError, ToolCatalogEntry, ToolCatalogService, WebUiAuthenticatedCaller,
};

pub(crate) struct RebornToolCatalogService {
    registry: Arc<ExtensionRegistry>,
}

impl RebornToolCatalogService {
    pub(crate) fn new(registry: Arc<ExtensionRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ToolCatalogService for RebornToolCatalogService {
    async fn list_tools(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<Vec<ToolCatalogEntry>, RebornServicesError> {
        Ok(self
            .registry
            .capabilities()
            .map(|descriptor| {
                // Map the manifest-declared default permission to the resolved
                // default tools-tab state. `Deny` is admin-locked: the user
                // cannot reconfigure it.
                let (default_state, locked) = match descriptor.default_permission {
                    PermissionMode::Allow => (ToolPermissionState::AlwaysAllow, false),
                    PermissionMode::Ask => (ToolPermissionState::AskEachTime, false),
                    PermissionMode::Deny => (ToolPermissionState::Disabled, true),
                };
                ToolCatalogEntry {
                    capability_id: descriptor.id.to_string(),
                    description: descriptor.description.clone(),
                    default_state,
                    locked,
                }
            })
            .collect())
    }
}
