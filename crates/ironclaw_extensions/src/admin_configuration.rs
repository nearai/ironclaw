//! Declarative instance-configuration vocabulary for extension manifests.
//!
//! These descriptors describe deployment-owned values only. They do not
//! install or execute an extension, and they do not create a caller's personal
//! OAuth or pairing binding.

use std::collections::BTreeSet;

use ironclaw_host_api::SecretHandle;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_CONFIGURATION_FIELDS: usize = 64;
const MAX_GROUP_ID_BYTES: usize = 128;

/// Stable identity for a configuration form shared by one or more manifests.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct AdminConfigurationGroupId(String);

impl AdminConfigurationGroupId {
    pub fn new(value: impl Into<String>) -> Result<Self, AdminConfigurationDescriptorError> {
        let value = value.into();
        validate_group_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AdminConfigurationGroupId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AdminConfigurationGroupId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// One deployment-owned value in a manifest-declared configuration form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfigurationField {
    pub handle: SecretHandle,
    pub label: String,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub required: bool,
}

/// One reusable operator form. Equal group ids must carry equal descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionAdminConfigurationDescriptor {
    pub group_id: AdminConfigurationGroupId,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub fields: Vec<AdminConfigurationField>,
}

impl ExtensionAdminConfigurationDescriptor {
    pub fn validate(&self) -> Result<(), AdminConfigurationDescriptorError> {
        if self.display_name.trim().is_empty() {
            return Err(AdminConfigurationDescriptorError::EmptyDisplayName);
        }
        if self.fields.is_empty() || self.fields.len() > MAX_CONFIGURATION_FIELDS {
            return Err(AdminConfigurationDescriptorError::FieldCount {
                count: self.fields.len(),
                max: MAX_CONFIGURATION_FIELDS,
            });
        }
        let mut seen = BTreeSet::new();
        for field in &self.fields {
            if field.label.trim().is_empty() {
                return Err(AdminConfigurationDescriptorError::EmptyFieldLabel {
                    handle: field.handle.clone(),
                });
            }
            if !seen.insert(field.handle.clone()) {
                return Err(AdminConfigurationDescriptorError::DuplicateField {
                    handle: field.handle.clone(),
                });
            }
        }
        Ok(())
    }
}

fn validate_group_id(value: &str) -> Result<(), AdminConfigurationDescriptorError> {
    if value.is_empty() || value.len() > MAX_GROUP_ID_BYTES {
        return Err(AdminConfigurationDescriptorError::InvalidGroupId);
    }
    let segments = value.split('.').collect::<Vec<_>>();
    if segments.len() < 2
        || segments.iter().any(|segment| {
            segment.is_empty()
                || !segment.as_bytes()[0].is_ascii_lowercase()
                || segment.bytes().any(|byte| {
                    !byte.is_ascii_lowercase()
                        && !byte.is_ascii_digit()
                        && !matches!(byte, b'_' | b'-')
                })
        })
    {
        return Err(AdminConfigurationDescriptorError::InvalidGroupId);
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AdminConfigurationDescriptorError {
    #[error("group_id must be a lowercase dotted identifier")]
    InvalidGroupId,
    #[error("display_name must not be empty")]
    EmptyDisplayName,
    #[error("fields must contain between 1 and {max} entries, got {count}")]
    FieldCount { count: usize, max: usize },
    #[error("field `{handle}` has an empty label")]
    EmptyFieldLabel { handle: SecretHandle },
    #[error("duplicate admin configuration field `{handle}`")]
    DuplicateField { handle: SecretHandle },
}
