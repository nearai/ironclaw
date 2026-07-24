use async_trait::async_trait;
use ironclaw_auth::{SecretCleanupReport, SecretCleanupRequest};
use ironclaw_host_api::ProductSurfaceError;

use crate::RebornProductAuthServices;

pub(crate) use ironclaw_extension_host::ExtensionCredentialCleanup;
pub(crate) type RebornLocalExtensionManagementPort =
    ironclaw_extension_host::ExtensionLifecycleManager;

#[cfg(test)]
pub(crate) mod hosted_mcp_test_support;

#[async_trait]
impl ExtensionCredentialCleanup for RebornProductAuthServices {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError> {
        RebornProductAuthServices::cleanup_credentials_for_lifecycle(self, request)
            .await
            .map_err(|error| {
                ProductSurfaceError::internal_from(format!(
                    "extension credential cleanup failed: {:?}",
                    error.code
                ))
            })
    }
}
