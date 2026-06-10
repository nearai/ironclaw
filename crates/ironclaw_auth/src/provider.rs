use std::fmt;

use async_trait::async_trait;
use ironclaw_host_api::SecretHandle;
use secrecy::{ExposeSecret, SecretString};

use crate::{
    AuthFlowId, AuthProductError, AuthProductScope, AuthorizationCodeHash, CredentialAccountId,
    CredentialAccountLabel, PkceVerifierHash, ProviderScope, ids::AuthProviderId,
};

macro_rules! one_shot_secret {
    ($name:ident, $label:literal) => {
        pub struct $name(SecretString);

        impl $name {
            pub fn new(value: SecretString) -> Result<Self, AuthProductError> {
                let exposed = value.expose_secret();
                if exposed.is_empty() {
                    return Err(AuthProductError::invalid_request(format!(
                        "{} must not be empty",
                        $label
                    )));
                }
                if exposed.trim() != exposed {
                    return Err(AuthProductError::invalid_request(format!(
                        "{} must not contain leading or trailing whitespace",
                        $label
                    )));
                }
                if exposed.chars().any(|c| c == '\0' || c.is_control()) {
                    return Err(AuthProductError::invalid_request(format!(
                        "{} must not contain NUL/control characters",
                        $label
                    )));
                }
                Ok(Self(value))
            }

            pub fn expose_secret(&self) -> &str {
                self.0.expose_secret()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }
    };
}

one_shot_secret!(OAuthAuthorizationCode, "oauth authorization code");
one_shot_secret!(PkceVerifierSecret, "pkce verifier");

/// One-shot provider exchange input. This type intentionally does not implement
/// serde traits because it may carry raw OAuth code and PKCE verifier material.
pub struct OAuthProviderCallbackRequest {
    pub provider: AuthProviderId,
    pub account_label: CredentialAccountLabel,
    pub authorization_code: OAuthAuthorizationCode,
    pub authorization_code_hash: AuthorizationCodeHash,
    pub pkce_verifier: PkceVerifierSecret,
    pub pkce_verifier_hash: PkceVerifierHash,
    pub scopes: Vec<ProviderScope>,
}

impl fmt::Debug for OAuthProviderCallbackRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthProviderCallbackRequest")
            .field("provider", &self.provider)
            .field("account_label", &self.account_label)
            .field("authorization_code", &"[REDACTED]")
            .field("authorization_code_hash", &self.authorization_code_hash)
            .field("pkce_verifier", &"[REDACTED]")
            .field("pkce_verifier_hash", &self.pkce_verifier_hash)
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// Provider-exchange context claimed by the product-auth flow before raw
/// provider material is exchanged or stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderExchangeContext {
    pub scope: AuthProductScope,
    pub flow_id: AuthFlowId,
}

/// Provider-exchange result safe to store in auth-flow/account records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderExchange {
    pub provider: AuthProviderId,
    pub account_label: CredentialAccountLabel,
    pub authorization_code_hash: AuthorizationCodeHash,
    pub pkce_verifier_hash: PkceVerifierHash,
    pub access_secret: SecretHandle,
    pub refresh_secret: Option<SecretHandle>,
    pub scopes: Vec<ProviderScope>,
    pub account_id: Option<CredentialAccountId>,
}

/// One-shot provider refresh input. This type intentionally does not implement
/// serde traits because refresh authority must stay behind host-mediated
/// credential/egress boundaries.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthProviderRefreshRequest {
    pub provider: AuthProviderId,
    pub scope: AuthProductScope,
    pub account_id: CredentialAccountId,
    pub refresh_secret: SecretHandle,
    pub scopes: Vec<ProviderScope>,
}

impl fmt::Debug for OAuthProviderRefreshRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthProviderRefreshRequest")
            .field("provider", &self.provider)
            .field("scope", &self.scope)
            .field("account_id", &self.account_id)
            .field("refresh_secret", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// Provider refresh result safe to store back into credential-account records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderRefresh {
    pub provider: AuthProviderId,
    pub access_secret: SecretHandle,
    pub refresh_secret: Option<SecretHandle>,
    pub scopes: Vec<ProviderScope>,
}

#[async_trait]
pub trait AuthProviderClient: Send + Sync {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError>;

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError>;

    async fn cleanup_exchange(
        &self,
        _context: OAuthProviderExchangeContext,
        _exchange: &OAuthProviderExchange,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

/// Provider client used when product-auth storage is available but no OAuth
/// provider implementation is configured for the process.
#[derive(Debug, Default)]
pub struct UnavailableAuthProviderClient;

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

pub fn validate_provider_callback_request(
    request: &OAuthProviderCallbackRequest,
) -> Result<(), AuthProductError> {
    if request.authorization_code.expose_secret().trim().is_empty()
        || request.pkce_verifier.expose_secret().trim().is_empty()
    {
        return Err(AuthProductError::MalformedCallback);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthFlowId, AuthProviderId, AuthSurface, CredentialAccountId, CredentialAccountLabel,
        OAuthAuthorizationCode, PkceVerifierSecret, ProviderScope, authorization_code_hash,
        pkce_verifier_hash,
    };
    use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, UserId};

    fn auth_scope() -> crate::AuthProductScope {
        crate::AuthProductScope::new(
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap(),
            AuthSurface::Web,
        )
    }

    #[tokio::test]
    async fn unavailable_auth_provider_client_validates_before_returning_backend_unavailable() {
        let client = UnavailableAuthProviderClient;
        let ctx = OAuthProviderExchangeContext {
            scope: auth_scope(),
            flow_id: AuthFlowId::new(),
        };
        let authorization_code =
            OAuthAuthorizationCode::new(secrecy::SecretString::from("real-code")).unwrap();
        let pkce_verifier =
            PkceVerifierSecret::new(secrecy::SecretString::from("real-verifier")).unwrap();
        let authorization_code_hash = authorization_code_hash(&authorization_code).unwrap();
        let pkce_verifier_hash = pkce_verifier_hash(&pkce_verifier).unwrap();
        let valid = OAuthProviderCallbackRequest {
            provider: AuthProviderId::new("google").unwrap(),
            account_label: CredentialAccountLabel::new("Alice Google").unwrap(),
            authorization_code,
            authorization_code_hash,
            pkce_verifier,
            pkce_verifier_hash,
            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
        };

        let err = client.exchange_callback(ctx, valid).await.unwrap_err();
        assert_eq!(err, AuthProductError::BackendUnavailable);

        let refresh_err = client
            .refresh_token(OAuthProviderRefreshRequest {
                provider: AuthProviderId::new("google").unwrap(),
                scope: auth_scope(),
                account_id: CredentialAccountId::new(),
                refresh_secret: SecretHandle::new("refresh").unwrap(),
                scopes: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(refresh_err, AuthProductError::BackendUnavailable);
    }

    #[tokio::test]
    async fn unavailable_auth_provider_client_rejects_malformed_callback_before_backend_unavailable()
     {
        let client = UnavailableAuthProviderClient;
        let ctx = OAuthProviderExchangeContext {
            scope: auth_scope(),
            flow_id: AuthFlowId::new(),
        };
        let authorization_code =
            OAuthAuthorizationCode::new(secrecy::SecretString::from("real-code")).unwrap();
        let pkce_verifier =
            PkceVerifierSecret::new(secrecy::SecretString::from("real-verifier")).unwrap();
        let authorization_code_hash = authorization_code_hash(&authorization_code).unwrap();
        let pkce_verifier_hash = pkce_verifier_hash(&pkce_verifier).unwrap();
        let malformed_code = OAuthProviderCallbackRequest {
            provider: AuthProviderId::new("google").unwrap(),
            account_label: CredentialAccountLabel::new("Alice Google").unwrap(),
            authorization_code: OAuthAuthorizationCode(secrecy::SecretString::from("")),
            authorization_code_hash: authorization_code_hash.clone(),
            pkce_verifier,
            pkce_verifier_hash: pkce_verifier_hash.clone(),
            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
        };

        let err = client
            .exchange_callback(ctx.clone(), malformed_code)
            .await
            .unwrap_err();
        assert_eq!(err, AuthProductError::MalformedCallback);

        let malformed_pkce = OAuthProviderCallbackRequest {
            provider: AuthProviderId::new("google").unwrap(),
            account_label: CredentialAccountLabel::new("Alice Google").unwrap(),
            authorization_code,
            authorization_code_hash,
            pkce_verifier: PkceVerifierSecret(secrecy::SecretString::from("")),
            pkce_verifier_hash,
            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
        };

        let err = client
            .exchange_callback(ctx, malformed_pkce)
            .await
            .unwrap_err();
        assert_eq!(err, AuthProductError::MalformedCallback);
    }
}
