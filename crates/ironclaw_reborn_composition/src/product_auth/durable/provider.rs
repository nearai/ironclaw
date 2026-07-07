use async_trait::async_trait;

use ironclaw_auth::{
    AuthProductError, AuthProviderClient, OAuthProviderCallbackRequest, OAuthProviderExchange,
    OAuthProviderExchangeContext, OAuthProviderRefresh, OAuthProviderRefreshRequest,
    validate_provider_callback_request,
};

/// Explicit provider client used when durable product-auth storage is available
/// but no OAuth provider implementation has been composed for this process.
#[derive(Debug, Default)]
pub(crate) struct UnavailableAuthProviderClient;

#[async_trait]
impl AuthProviderClient for UnavailableAuthProviderClient {
    async fn exchange_callback(
        &self,
        _context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        validate_provider_callback_request(&request)?;
        Err(AuthProductError::BackendUnavailable)
    }

    async fn refresh_token(
        &self,
        _request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }
}
