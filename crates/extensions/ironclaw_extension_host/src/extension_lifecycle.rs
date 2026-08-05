use async_trait::async_trait;
use ironclaw_auth::{SecretCleanupReport, SecretCleanupRequest};
use ironclaw_product_contracts::surface::ProductSurfaceError;
use std::sync::Arc;

use ironclaw_auth::RebornProductAuthServices;

pub use ironclaw_extension_host::ExtensionCredentialCleanup;
pub type RebornLocalExtensionManagementPort = ironclaw_extension_host::ExtensionLifecycleManager;

#[cfg(any(test, feature = "test-support"))]
pub mod hosted_mcp_test_support;

pub struct RebornProductAuthCredentialCleanup {
    product_auth: Arc<RebornProductAuthServices>,
}

impl RebornProductAuthCredentialCleanup {
    pub fn new(product_auth: Arc<RebornProductAuthServices>) -> Self {
        Self { product_auth }
    }
}

#[async_trait]
impl ExtensionCredentialCleanup for RebornProductAuthCredentialCleanup {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError> {
        RebornProductAuthServices::cleanup_credentials_for_lifecycle(&self.product_auth, request)
            .await
            .map_err(|error| {
                ProductSurfaceError::internal_from(format!(
                    "extension credential cleanup failed: {:?}",
                    error.code
                ))
            })
    }
}
