//! Validated newtypes for the external-identity key parts.
//!
//! `(tenant_id, surface_kind, provider_kind, provider_instance_id,
//! external_subject_id)` is the canonical key. `tenant_id` is already a
//! typed [`TenantId`](ironclaw_host_api::TenantId) and `surface_kind` is the
//! [`SurfaceKind`](crate::SurfaceKind) enum; this module gives the remaining
//! three adapter-supplied parts the same treatment so they cross the
//! resolver boundary as specialized types rather than raw `&str`
//! (`.claude/rules/types.md` — identifiers become newtypes at the earliest
//! internal boundary). Validation mirrors the sibling
//! `RebornIdentityProviderId` / `RebornIdentityProviderUserId` newtypes:
//! non-empty and free of control characters.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Rejection reason when constructing an identity key part.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid identity key part `{field}`: {reason}")]
pub struct IdentityKeyError {
    pub field: &'static str,
    pub reason: &'static str,
}

fn validate(field: &'static str, value: &str) -> Result<(), IdentityKeyError> {
    if value.is_empty() {
        return Err(IdentityKeyError {
            field,
            reason: "must not be empty",
        });
    }
    if value.chars().any(|character| character.is_control()) {
        return Err(IdentityKeyError {
            field,
            reason: "must not contain control characters",
        });
    }
    Ok(())
}

macro_rules! identity_key_newtype {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        ///
        /// Follows the canonical validated-newtype shape from
        /// `.claude/rules/types.md`: wire deserialization (`try_from =
        /// "String"`) runs the same `validate` as `::new`, so the invariant
        /// holds across construction and serialization, and the only
        /// boundary crossings are the explicit `as_str` / `as_ref` /
        /// `into_inner` accessors (no `From<String>` / `Deref<str>`, which
        /// would silently bypass validation).
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            /// Construct after validating (non-empty, no control characters).
            pub fn new(value: impl Into<String>) -> Result<Self, IdentityKeyError> {
                let value = value.into();
                validate($field, &value)?;
                Ok(Self(value))
            }

            /// Borrow the underlying string for storage / query binding.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume the newtype, returning the owned underlying string.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentityKeyError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                validate($field, &value)?;
                Ok(Self(value))
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
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

identity_key_newtype!(
    /// Provider name key part (`google`, `github`, `telegram`, `slack`, …).
    ProviderKind,
    "provider_kind"
);
identity_key_newtype!(
    /// Adapter installation / instance id key part (channel actors); absent
    /// for surfaces without an installation (browser OAuth login).
    ProviderInstanceId,
    "provider_instance_id"
);
identity_key_newtype!(
    /// Stable per-provider subject id key part (OAuth `sub`, channel actor id).
    ExternalSubjectId,
    "external_subject_id"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_control_characters() {
        assert!(ProviderKind::new("").is_err());
        assert!(ExternalSubjectId::new("with\nnewline").is_err());
        assert_eq!(
            ProviderKind::new("google").expect("valid").as_str(),
            "google"
        );
        assert_eq!(
            ProviderInstanceId::new("install-1")
                .expect("valid")
                .as_str(),
            "install-1"
        );
    }

    #[test]
    fn serde_round_trips_and_revalidates_on_the_wire() {
        let provider = ProviderKind::new("google").expect("valid");
        let json = serde_json::to_string(&provider).expect("serialize");
        assert_eq!(json, "\"google\"", "serializes as the bare inner string");
        let back: ProviderKind = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, provider);

        // Wire deserialization runs the same validator as `::new`, so an
        // invalid persisted value is rejected rather than silently admitted.
        assert!(serde_json::from_str::<ProviderKind>("\"\"").is_err());
        assert!(serde_json::from_str::<ExternalSubjectId>("\"with\\nnewline\"").is_err());
    }

    #[test]
    fn into_inner_and_as_ref_expose_the_value() {
        let subject = ExternalSubjectId::new("sub-7").expect("valid");
        let borrowed: &str = subject.as_ref();
        assert_eq!(borrowed, "sub-7");
        assert_eq!(subject.into_inner(), "sub-7");
    }
}
