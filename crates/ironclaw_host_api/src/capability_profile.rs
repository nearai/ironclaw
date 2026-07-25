//! Relative schema references for capability operation contracts.
//!
//! [`CapabilityProfileSchemaRef`] is the validated newtype every capability
//! uses for its `input_schema_ref` / `output_schema_ref`. (The former
//! host-defined "capability profile" contract vocabulary — profile ids and
//! required-operation contracts extensions could claim to `implements` — was
//! removed once the memory adapter trait became the operation contract; only
//! this schema-ref newtype remains.)

use serde::{Deserialize, Serialize};

use crate::HostApiError;

fn validate_schema_ref(value: &str) -> Result<(), HostApiError> {
    if value.is_empty() {
        return Err(HostApiError::invalid_path(value, "must not be empty"));
    }
    if value.len() > 512 {
        return Err(HostApiError::invalid_path(
            value,
            "must be at most 512 bytes",
        ));
    }
    if value.starts_with('/') {
        return Err(HostApiError::invalid_path(value, "must be relative"));
    }
    if value.contains('\\') {
        return Err(HostApiError::invalid_path(
            value,
            "backslashes are not allowed",
        ));
    }
    if value.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return Err(HostApiError::invalid_path(
            value,
            "NUL/control characters are not allowed",
        ));
    }
    for ch in value.chars() {
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/')) {
            return Err(HostApiError::invalid_path(
                value,
                "only ASCII alphanumerics, '.', '_', '-', and '/' are allowed",
            ));
        }
    }
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(HostApiError::invalid_path(
                value,
                "empty and dot path segments are not allowed",
            ));
        }
    }
    Ok(())
}

/// Relative schema reference used by a capability operation contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityProfileSchemaRef(String);

impl CapabilityProfileSchemaRef {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_schema_ref(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for CapabilityProfileSchemaRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for CapabilityProfileSchemaRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CapabilityProfileSchemaRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}
