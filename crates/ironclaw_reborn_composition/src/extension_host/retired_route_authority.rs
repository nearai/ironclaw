//! Installation-backed [`RetiredChannelRouteAuthority`]: the durable answer to
//! "can the run-delivery router ever see a handler for this adapter again?".
//!
//! A channel adapter id doubles as its extension/installation id (one channel
//! surface per extension), so a missing installation row is the fail-closed
//! proof that the route is retired — a merely-unregistered handler during
//! startup still has its installation row and stays pending. See
//! `ironclaw_product::RunDeliveryEventRouter::set_retired_route_authority`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{ExtensionInstallationId, ExtensionInstallationStore};
use ironclaw_product::{ProductWorkflowError, RetiredChannelRouteAuthority};

pub(crate) struct InstallationBackedRetiredRouteAuthority {
    installations: Arc<dyn ExtensionInstallationStore>,
}

impl InstallationBackedRetiredRouteAuthority {
    pub(crate) fn new(installations: Arc<dyn ExtensionInstallationStore>) -> Self {
        Self { installations }
    }
}

#[async_trait]
impl RetiredChannelRouteAuthority for InstallationBackedRetiredRouteAuthority {
    async fn channel_route_is_retired(
        &self,
        adapter_id: &str,
    ) -> Result<bool, ProductWorkflowError> {
        let installation_id =
            ExtensionInstallationId::new(adapter_id.to_string()).map_err(|error| {
                ProductWorkflowError::Transient {
                    reason: format!("retired-route adapter id is invalid: {error}"),
                }
            })?;
        let installation = self
            .installations
            .get_installation(&installation_id)
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("retired-route installation read failed: {error}"),
            })?;
        Ok(installation.is_none())
    }
}
