//! Host-defined capability profile contracts.
//!
//! Capability profiles are portable, host-defined contracts such as
//! `memory.context_retrieval.v1`. Extensions may later claim that their
//! provider-prefixed capabilities implement these operations, but this module
//! only defines the neutral contract vocabulary.

use serde::{Deserialize, Serialize};

use crate::{
    HostApiError,
    dotted_id::{PrefixRule, VersionRule, validate_dotted_id},
};

fn validate_versioned_dotted_id(kind: &'static str, value: &str) -> Result<(), HostApiError> {
    validate_dotted_id(
        kind,
        value,
        3,
        "must have at least domain, name, and version segments",
        PrefixRule::Any,
        VersionRule::Versioned,
    )
}

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

macro_rules! string_contract_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
                let value = value.into();
                validate_versioned_dotted_id($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

string_contract_id!(CapabilityProfileId, "capability_profile");
string_contract_id!(CapabilityProfileOperationId, "capability_profile_operation");

/// Relative schema reference used by a host-defined profile operation contract.
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

/// One required operation for a host-defined capability profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProfileOperationContract {
    id: CapabilityProfileOperationId,
    input_schema_ref: CapabilityProfileSchemaRef,
    output_schema_ref: CapabilityProfileSchemaRef,
}

impl CapabilityProfileOperationContract {
    pub fn new(
        id: CapabilityProfileOperationId,
        input_schema_ref: impl Into<String>,
        output_schema_ref: impl Into<String>,
    ) -> Result<Self, HostApiError> {
        Ok(Self {
            id,
            input_schema_ref: CapabilityProfileSchemaRef::new(input_schema_ref)?,
            output_schema_ref: CapabilityProfileSchemaRef::new(output_schema_ref)?,
        })
    }

    pub fn id(&self) -> &CapabilityProfileOperationId {
        &self.id
    }

    pub fn input_schema_ref(&self) -> &CapabilityProfileSchemaRef {
        &self.input_schema_ref
    }

    pub fn output_schema_ref(&self) -> &CapabilityProfileSchemaRef {
        &self.output_schema_ref
    }
}

/// Host-defined portability contract that extensions may claim to implement.
///
/// Required operations are kept sorted by id so equality and serialization are
/// order-independent across construction sites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityProfileContract {
    id: CapabilityProfileId,
    required_operations: Vec<CapabilityProfileOperationContract>,
}

impl<'de> Deserialize<'de> for CapabilityProfileContract {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Helper {
            id: CapabilityProfileId,
            required_operations: Vec<CapabilityProfileOperationContract>,
        }
        let helper = Helper::deserialize(deserializer)?;
        CapabilityProfileContract::new(helper.id, helper.required_operations)
            .map_err(serde::de::Error::custom)
    }
}

impl CapabilityProfileContract {
    pub fn new(
        id: CapabilityProfileId,
        mut required_operations: Vec<CapabilityProfileOperationContract>,
    ) -> Result<Self, HostApiError> {
        if required_operations.is_empty() {
            return Err(HostApiError::invariant(
                "capability profile must require at least one operation",
            ));
        }
        required_operations.sort_by(|a, b| a.id.cmp(&b.id));
        for window in required_operations.windows(2) {
            if window[0].id == window[1].id {
                return Err(HostApiError::invariant(format!(
                    "duplicate capability profile operation {}",
                    window[0].id
                )));
            }
        }
        Ok(Self {
            id,
            required_operations,
        })
    }

    pub fn id(&self) -> &CapabilityProfileId {
        &self.id
    }

    pub fn required_operations(&self) -> &[CapabilityProfileOperationContract] {
        &self.required_operations
    }
}
