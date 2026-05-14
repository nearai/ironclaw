//! Host-port vocabulary contracts.
//!
//! Host ports name mediated host APIs that a capability implementation may use
//! after authorization and obligation preparation. This module only defines the
//! shared vocabulary and scoped view shape; concrete port implementations live in
//! host/runtime service crates.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    HostApiError,
    dotted_id::{PrefixRule, VersionRule, validate_dotted_id},
};

fn validate_dotted_host_port_id(value: &str) -> Result<(), HostApiError> {
    validate_dotted_id(
        "host_port",
        value,
        3,
        "must have at least host, domain, and service segments",
        PrefixRule::Required("host."),
        VersionRule::Unversioned,
    )
}

/// Stable identifier for a host-mediated API surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HostPortId(String);

impl HostPortId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_dotted_host_port_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for HostPortId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for HostPortId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for HostPortId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// One host port granted into a scoped invocation view.
///
/// This is intentionally a thin grant token. Future scoped or attenuated host-port
/// grants should use a distinct wire type rather than overloading this catalog
/// reference shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostPortGrant {
    id: HostPortId,
}

impl HostPortGrant {
    pub fn new(id: HostPortId) -> Self {
        Self { id }
    }

    pub fn id(&self) -> &HostPortId {
        &self.id
    }
}

/// Host-defined catalog entry for one known host port.
///
/// A catalog entry names a contract that manifest validation may reference. It
/// does not create, own, or dispatch a concrete host-port implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostPortCatalogEntry {
    id: HostPortId,
}

impl HostPortCatalogEntry {
    pub fn new(id: HostPortId) -> Self {
        Self { id }
    }

    pub fn id(&self) -> &HostPortId {
        &self.id
    }
}

/// Host-defined catalog of known host-port contract names.
///
/// The catalog is validation vocabulary only. Runtime service crates decide how
/// to construct concrete scoped adapters after authorization and obligation
/// handling. Entries are kept sorted by id so equality and serialization are
/// order-independent across construction sites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostPortCatalog {
    entries: Vec<HostPortCatalogEntry>,
}

impl HostPortCatalog {
    pub fn new(mut entries: Vec<HostPortCatalogEntry>) -> Result<Self, HostApiError> {
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        for window in entries.windows(2) {
            if window[0].id == window[1].id {
                return Err(HostApiError::invariant(format!(
                    "duplicate host port catalog entry {}",
                    window[0].id
                )));
            }
        }
        Ok(Self { entries })
    }

    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn entries(&self) -> &[HostPortCatalogEntry] {
        &self.entries
    }

    pub fn contains(&self, id: &HostPortId) -> bool {
        self.entries
            .binary_search_by(|entry| entry.id.cmp(id))
            .is_ok()
    }

    /// Return every required port that is not present in the catalog, in input
    /// order, with duplicates removed.
    pub fn missing_required<'a, I>(&self, required: I) -> Vec<HostPortId>
    where
        I: IntoIterator<Item = &'a HostPortId>,
    {
        let mut seen = BTreeSet::new();
        let mut missing: Vec<HostPortId> = Vec::new();
        for id in required {
            if !self.contains(id) && seen.insert(id.clone()) {
                missing.push(id.clone());
            }
        }
        missing
    }

    pub fn validate_required<'a, I>(&self, required: I) -> Result<(), HostApiError>
    where
        I: IntoIterator<Item = &'a HostPortId>,
    {
        let missing = self.missing_required(required);
        if missing.is_empty() {
            return Ok(());
        }
        let names = missing
            .iter()
            .map(HostPortId::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        Err(HostApiError::invariant(format!(
            "unknown host ports {names}"
        )))
    }
}

impl Default for HostPortCatalog {
    fn default() -> Self {
        Self::empty()
    }
}

impl<'de> Deserialize<'de> for HostPortCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Helper {
            entries: Vec<HostPortCatalogEntry>,
        }
        let helper = Helper::deserialize(deserializer)?;
        HostPortCatalog::new(helper.entries).map_err(serde::de::Error::custom)
    }
}

/// Scoped set of host ports available to an invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostPortView {
    grants: Vec<HostPortGrant>,
}

impl<'de> Deserialize<'de> for HostPortView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Helper {
            grants: Vec<HostPortGrant>,
        }
        let helper = Helper::deserialize(deserializer)?;
        HostPortView::new(helper.grants).map_err(serde::de::Error::custom)
    }
}

impl HostPortView {
    pub fn new(mut grants: Vec<HostPortGrant>) -> Result<Self, HostApiError> {
        grants.sort_by(|a, b| a.id.cmp(&b.id));
        for window in grants.windows(2) {
            if window[0].id == window[1].id {
                return Err(HostApiError::invariant(format!(
                    "duplicate host port grant {}",
                    window[0].id
                )));
            }
        }
        Ok(Self { grants })
    }

    pub fn empty() -> Self {
        Self { grants: Vec::new() }
    }

    pub fn grants(&self) -> &[HostPortGrant] {
        &self.grants
    }

    pub fn allows(&self, id: &HostPortId) -> bool {
        self.grants
            .binary_search_by(|grant| grant.id.cmp(id))
            .is_ok()
    }

    pub fn allows_all<'a, I>(&self, required: I) -> bool
    where
        I: IntoIterator<Item = &'a HostPortId>,
    {
        required.into_iter().all(|id| self.allows(id))
    }
}

impl Default for HostPortView {
    fn default() -> Self {
        Self::empty()
    }
}
