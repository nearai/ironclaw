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
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
}
