//! Authority-bearing identifier contracts.
//!
//! This module defines the newtypes used to prevent stringly-typed authority
//! flow: tenant/user/agent/project/thread scope IDs, extension and capability IDs,
//! secret handles, and UUID-backed invocation/process/grant/reservation/audit
//! IDs. Constructors validate path-adjacent strings so invalid names cannot be
//! smuggled into manifests, mount paths, approvals, or audit records.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::HostApiError;

fn has_forbidden_control(value: &str) -> bool {
    value.chars().any(|c| c == '\0' || c.is_control())
}

fn validate_scope_id(kind: &'static str, value: &str) -> Result<(), HostApiError> {
    if value.is_empty() {
        return Err(HostApiError::invalid_id(kind, value, "must not be empty"));
    }
    if value.len() > 256 {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "must be at most 256 bytes",
        ));
    }
    if value == "." || value == ".." {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "dot segments are not allowed",
        ));
    }
    if value.contains('/') || value.contains('\\') {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "path separators are not allowed",
        ));
    }
    if has_forbidden_control(value) {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "NUL/control characters are not allowed",
        ));
    }
    Ok(())
}

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
}

fn validate_name_segment(kind: &'static str, value: &str) -> Result<(), HostApiError> {
    if value.is_empty() {
        return Err(HostApiError::invalid_id(kind, value, "must not be empty"));
    }
    if value.len() > 128 {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "must be at most 128 bytes",
        ));
    }
    let first = value.as_bytes()[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "must start with lowercase ASCII letter or digit",
        ));
    }
    if value == "." || value == ".." || value.contains("..") {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "dot-dot segments are not allowed",
        ));
    }
    if value.bytes().any(|b| !(is_name_char(b) || b == b'.')) {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "only lowercase ASCII letters, digits, '_', '-', and '.' are allowed",
        ));
    }
    if value.split('.').any(str::is_empty) {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "empty dot segments are not allowed",
        ));
    }
    Ok(())
}

macro_rules! string_id {
    ($name:ident, $kind:literal, $validator:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
                let value = value.into();
                $validator($kind, &value)?;
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

            pub fn parse(value: &str) -> Result<Self, uuid::Error> {
                Uuid::parse_str(value).map(Self)
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

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

string_id!(TenantId, "tenant", validate_scope_id);
string_id!(UserId, "user", validate_scope_id);
string_id!(AgentId, "agent", validate_scope_id);
string_id!(ProjectId, "project", validate_scope_id);
string_id!(MissionId, "mission", validate_scope_id);
string_id!(ThreadId, "thread", validate_scope_id);
string_id!(ExtensionId, "extension", validate_name_segment);
string_id!(PackageId, "package", validate_name_segment);
string_id!(SecretHandle, "secret", validate_name_segment);
string_id!(SystemServiceId, "system_service", validate_name_segment);

/// Extension-prefixed capability identifier.
///
/// Capability IDs require at least two dot-separated segments and may use
/// additional segments for namespacing, e.g. `github.issues.search`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityId(String);

impl CapabilityId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        if value.is_empty() || !value.contains('.') {
            return Err(HostApiError::invalid_id(
                "capability",
                value,
                "must be '<extension>.<capability>[.<sub>...]'",
            ));
        }
        if value.split('.').count() < 2 || value.split('.').any(str::is_empty) {
            return Err(HostApiError::invalid_id(
                "capability",
                value,
                "empty dot segments are not allowed",
            ));
        }
        for segment in value.split('.') {
            validate_name_segment("capability", segment)?;
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for CapabilityId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CapabilityId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

uuid_id!(InvocationId);
uuid_id!(ProcessId);
uuid_id!(CapabilityGrantId);
uuid_id!(ResourceReservationId);
uuid_id!(ApprovalRequestId);
uuid_id!(AuditEventId);
uuid_id!(CorrelationId);
