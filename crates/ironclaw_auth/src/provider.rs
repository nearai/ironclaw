use std::fmt;

use async_trait::async_trait;
use ironclaw_host_api::SecretHandle;
use secrecy::{ExposeSecret, SecretString};

use crate::{
    AuthProductError, AuthorizationCodeHash, CredentialAccountId, CredentialAccountLabel,
    PkceVerifierHash, ProviderScope, ids::AuthProviderId,
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

            pub(crate) fn expose_secret(&self) -> &str {
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

#[async_trait]
pub trait AuthProviderClient: Send + Sync {
    async fn exchange_callback(
        &self,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError>;
}

pub(crate) fn validate_provider_callback_request(
    request: &OAuthProviderCallbackRequest,
) -> Result<(), AuthProductError> {
    if request.authorization_code.expose_secret().trim().is_empty()
        || request.pkce_verifier.expose_secret().trim().is_empty()
    {
        return Err(AuthProductError::MalformedCallback);
    }
    Ok(())
}
