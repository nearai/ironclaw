use std::{fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_auth::{AuthFlowId, AuthProductError, CredentialAccountId};
use ironclaw_host_api::{ResourceScope, SecretHandle};
use ironclaw_secrets::SecretStore;
use secrecy::SecretString;

/// Boundary for turning Google token material into durable secret handles.
#[async_trait]
pub(super) trait GoogleProviderTokenSink: Send + Sync {
    async fn store_tokens(
        &self,
        request: GoogleProviderTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError>;

    async fn store_refreshed_tokens(
        &self,
        request: GoogleProviderRefreshTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError>;

    async fn load_refresh_token(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretString, AuthProductError>;

    async fn delete_tokens(
        &self,
        scope: &ResourceScope,
        handles: &[SecretHandle],
    ) -> Result<(), AuthProductError>;
}

/// Raw Google token material passed exactly once to the injected storage
/// boundary. This type intentionally does not implement serde.
pub(super) struct GoogleProviderTokenSet {
    pub(super) access_token: SecretString,
    pub(super) refresh_token: Option<SecretString>,
}

impl fmt::Debug for GoogleProviderTokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderTokenSet")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Scoped token-storage request. Raw provider token material must be bound to
/// the already-claimed callback scope and flow before it reaches storage.
pub(super) struct GoogleProviderTokenStorageRequest {
    pub(super) scope: ResourceScope,
    pub(super) flow_id: AuthFlowId,
    pub(super) tokens: GoogleProviderTokenSet,
}

impl fmt::Debug for GoogleProviderTokenStorageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderTokenStorageRequest")
            .field("scope", &self.scope)
            .field("flow_id", &self.flow_id)
            .field("tokens", &self.tokens)
            .finish()
    }
}

/// Scoped token-storage request for account refresh. Raw token material is
/// passed exactly once to storage, and returned values are handles only.
pub(super) struct GoogleProviderRefreshTokenStorageRequest {
    pub(super) scope: ResourceScope,
    pub(super) account_id: CredentialAccountId,
    pub(super) tokens: GoogleProviderTokenSet,
}

impl fmt::Debug for GoogleProviderRefreshTokenStorageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderRefreshTokenStorageRequest")
            .field("scope", &self.scope)
            .field("account_id", &self.account_id)
            .field("tokens", &self.tokens)
            .finish()
    }
}

/// Durable secret handles produced after Google OAuth token material is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GoogleProviderStoredTokens {
    pub(super) access_secret: SecretHandle,
    pub(super) refresh_secret: Option<SecretHandle>,
}

pub(super) struct SecretStoreGoogleTokenSink {
    pub(super) store: Arc<dyn SecretStore>,
}

#[async_trait]
impl GoogleProviderTokenSink for SecretStoreGoogleTokenSink {
    async fn store_tokens(
        &self,
        request: GoogleProviderTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
        let access_secret = google_token_handle(&request, "access")?;
        let refresh_handle = request
            .tokens
            .refresh_token
            .as_ref()
            .map(|_| google_token_handle(&request, "refresh"))
            .transpose()?;
        let GoogleProviderTokenStorageRequest {
            scope,
            tokens,
            flow_id: _,
        } = request;
        let GoogleProviderTokenSet {
            access_token,
            refresh_token,
        } = tokens;
        self.store
            .put(scope.clone(), access_secret.clone(), access_token)
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;

        let refresh_secret = match refresh_token {
            Some(refresh_token) => {
                let handle = refresh_handle.ok_or(AuthProductError::BackendUnavailable)?;
                if let Err(error) = self
                    .store
                    .put(scope.clone(), handle.clone(), refresh_token)
                    .await
                {
                    let _ = self.store.delete(&scope, &access_secret).await;
                    return Err(map_secret_store_error(error));
                }
                Some(handle)
            }
            None => None,
        };

        Ok(GoogleProviderStoredTokens {
            access_secret,
            refresh_secret,
        })
    }

    async fn store_refreshed_tokens(
        &self,
        request: GoogleProviderRefreshTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
        let access_secret = google_refresh_token_handle(&request, "access")?;
        let refresh_handle = request
            .tokens
            .refresh_token
            .as_ref()
            .map(|_| google_refresh_token_handle(&request, "refresh"))
            .transpose()?;
        let GoogleProviderRefreshTokenStorageRequest {
            scope,
            tokens,
            account_id: _,
        } = request;
        let GoogleProviderTokenSet {
            access_token,
            refresh_token,
        } = tokens;
        self.store
            .put(scope.clone(), access_secret.clone(), access_token)
            .await
            .map_err(map_secret_store_error)?;

        let refresh_secret = match refresh_token {
            Some(refresh_token) => {
                let handle = refresh_handle.ok_or(AuthProductError::BackendUnavailable)?;
                if let Err(error) = self
                    .store
                    .put(scope.clone(), handle.clone(), refresh_token)
                    .await
                {
                    let _ = self.store.delete(&scope, &access_secret).await;
                    return Err(map_secret_store_error(error));
                }
                Some(handle)
            }
            None => None,
        };

        Ok(GoogleProviderStoredTokens {
            access_secret,
            refresh_secret,
        })
    }

    async fn load_refresh_token(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretString, AuthProductError> {
        let lease = self
            .store
            .lease_once(scope, handle)
            .await
            .map_err(map_refresh_secret_error)?;
        self.store
            .consume(scope, lease.id)
            .await
            .map_err(map_refresh_secret_error)
    }

    async fn delete_tokens(
        &self,
        scope: &ResourceScope,
        handles: &[SecretHandle],
    ) -> Result<(), AuthProductError> {
        for handle in handles {
            self.store
                .delete(scope, handle)
                .await
                .map_err(map_secret_store_error)?;
        }
        Ok(())
    }
}

fn google_token_handle(
    request: &GoogleProviderTokenStorageRequest,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "google-oauth-{token_kind}-{}-{}",
        request.flow_id, request.scope.invocation_id
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

fn google_refresh_token_handle(
    request: &GoogleProviderRefreshTokenStorageRequest,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "google-oauth-refresh-{token_kind}-{}-{}",
        request.account_id, request.scope.invocation_id
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

fn map_secret_store_error(_error: ironclaw_secrets::SecretStoreError) -> AuthProductError {
    AuthProductError::BackendUnavailable
}

fn map_refresh_secret_error(error: ironclaw_secrets::SecretStoreError) -> AuthProductError {
    if error.is_unknown_secret()
        || error.is_unknown_lease()
        || error.is_consumed()
        || error.is_revoked()
        || error.is_expired()
    {
        AuthProductError::RefreshFailed
    } else {
        AuthProductError::BackendUnavailable
    }
}
