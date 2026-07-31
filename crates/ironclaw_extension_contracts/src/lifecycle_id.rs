//! Bounded package-identity newtypes shared by the extension and product tiers.
//!
//! `LifecyclePackageId` sits here rather than beside the lifecycle
//! *projections* (`ironclaw_product_contracts::package_lifecycle`, PROPOSAL
//! §6.1.3) because `hosted_mcp` names it structurally — `RegisterHostedMcpRequest`
//! carries one and `hosted_mcp_extension_id` derives from one — and
//! `hosted_mcp` is extension-tier by its own charter. The dependency runs one
//! way (`product_contracts` → `extension_contracts`), so an id that both tiers
//! need can only live on this side of it.
//!
//! What stayed with the projections: `LifecycleProductAction`, the summaries,
//! the readiness blockers, and everything else product-facing.
//! `LifecycleBlockerRef` came along only because it shares this module's
//! bounded-string template; duplicating the template to split them would trade
//! one home for two.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use ironclaw_host_api::error::HostApiError;

/// Maximum byte length of a bounded lifecycle id.
pub const LIFECYCLE_ID_MAX_BYTES: usize = 256;
const LIFECYCLE_REF_MAX_BYTES: usize = 512;

macro_rules! bounded_lifecycle_string {
    ($name:ident, $kind:literal, $label:literal, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
                validate_lifecycle_string(value.into(), $kind, $label, $max).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

bounded_lifecycle_string!(
    LifecyclePackageId,
    "lifecycle_package",
    "lifecycle package id",
    LIFECYCLE_ID_MAX_BYTES
);
bounded_lifecycle_string!(
    LifecycleBlockerRef,
    "lifecycle_blocker",
    "lifecycle blocker ref",
    LIFECYCLE_REF_MAX_BYTES
);

fn validate_lifecycle_string(
    value: String,
    kind: &'static str,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, HostApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            format!("{label} must not be empty"),
        ));
    }
    if value.len() > max_bytes {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            format!("{label} must be at most {max_bytes} bytes"),
        ));
    }
    if trimmed.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            format!("{label} must not contain NUL/control characters"),
        ));
    }
    Ok(trimmed.to_string())
}
