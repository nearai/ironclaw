use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AuthProductError, validate_public_text};

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

macro_rules! string_newtype {
    ($name:ident, $validate:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
                Ok(Self($validate(value.into())?))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = AuthProductError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

macro_rules! validated_string {
    ($name:ident, $label:literal, $max:expr) => {
        string_newtype!($name, |value| validate_public_text(value, $label, $max));
    };
}

macro_rules! digest_string {
    ($name:ident, $label:literal) => {
        string_newtype!($name, |value| validate_digest_text(value, $label));
    };
}

fn validate_digest_text(value: String, label: &'static str) -> Result<String, AuthProductError> {
    let value = validate_public_text(value, label, 64)?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AuthProductError::invalid_request(format!(
            "{label} must be a 64-character hex digest"
        )));
    }
    Ok(value)
}

/// HTTPS authorization URL emitted to product surfaces for OAuth redirects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OAuthAuthorizationUrl(String);

impl OAuthAuthorizationUrl {
    const MAX_BYTES: usize = 2048;

    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        let value = validate_public_text(value, "oauth authorization url", Self::MAX_BYTES)?;
        validate_https_authorization_url(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for OAuthAuthorizationUrl {
    type Error = AuthProductError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<OAuthAuthorizationUrl> for String {
    fn from(value: OAuthAuthorizationUrl) -> Self {
        value.0
    }
}

impl fmt::Display for OAuthAuthorizationUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn validate_https_authorization_url(value: &str) -> Result<(), AuthProductError> {
    let parsed = url::Url::parse(value).map_err(|_| {
        AuthProductError::invalid_request("oauth authorization url must be a valid absolute url")
    })?;
    if parsed.scheme() != "https" {
        return Err(AuthProductError::invalid_request(
            "oauth authorization url must use https",
        ));
    }
    if parsed.host_str().is_none() {
        return Err(AuthProductError::invalid_request(
            "oauth authorization url host is required",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AuthProductError::invalid_request(
            "oauth authorization url must not contain userinfo",
        ));
    }
    Ok(())
}

uuid_id!(AuthFlowId);
uuid_id!(CredentialAccountId);
uuid_id!(AuthInteractionId);

validated_string!(AuthProviderId, "auth provider id", 128);
validated_string!(CredentialAccountLabel, "credential account label", 256);
validated_string!(ProviderScope, "provider scope", 256);
validated_string!(ProductActionRef, "product action ref", 256);
validated_string!(LifecyclePackageRef, "lifecycle package ref", 256);
validated_string!(TurnRunRef, "turn run ref", 256);
validated_string!(AuthGateRef, "auth gate ref", 256);
validated_string!(AuthSessionId, "auth session id", 256);
digest_string!(OpaqueStateHash, "opaque state hash");
digest_string!(PkceVerifierHash, "pkce verifier hash");
digest_string!(AuthorizationCodeHash, "authorization code hash");
