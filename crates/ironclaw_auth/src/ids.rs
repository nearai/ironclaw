use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AuthProductError, validate_public_text};

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

macro_rules! validated_string {
    ($name:ident, $label:literal, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
                Ok(Self(validate_public_text(value, $label, $max)?))
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

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::try_from(value).map_err(serde::de::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

uuid_id!(AuthFlowId);
uuid_id!(CredentialAccountId);
uuid_id!(AuthInteractionId);

validated_string!(AuthProviderId, "auth provider id", 128);
validated_string!(CredentialAccountLabel, "credential account label", 256);
validated_string!(ProductActionRef, "product action ref", 256);
validated_string!(LifecyclePackageRef, "lifecycle package ref", 256);
validated_string!(TurnRunRef, "turn run ref", 256);
validated_string!(AuthGateRef, "auth gate ref", 256);
validated_string!(AuthSessionId, "auth session id", 256);
validated_string!(OpaqueStateHash, "opaque state hash", 256);
validated_string!(PkceVerifierHash, "pkce verifier hash", 256);
validated_string!(AuthorizationCodeHash, "authorization code hash", 256);
