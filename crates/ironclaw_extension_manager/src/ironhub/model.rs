use std::{collections::BTreeMap, time::Duration};

use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::package_lifecycle::LifecycleProductResponse;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub(crate) const DEFAULT_IRONHUB_MANIFEST_URL: &str =
    "https://hub.ironclaw.com/api/catalog/manifest.json";
pub(crate) const MANIFEST_VERIFY_KEYS: &[(&str, &str)] = &[(
    "5895a21abea89672",
    "f64d2d3a3228b16ca59450364d26b278071a1a425544f242504033341d8459bd",
)];
pub(crate) const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_SIGNED_MANIFEST_BYTES: u64 = MAX_MANIFEST_BYTES * 2;
pub(crate) const MAX_METADATA_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_WASM_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_MANIFEST_AGE_SECS: i64 = 30 * 24 * 60 * 60;
pub(crate) const MAX_MANIFEST_FUTURE_SKEW_SECS: i64 = 5 * 60;
pub(crate) const MANIFEST_CACHE_TTL: Duration = Duration::from_secs(60);
pub(crate) const MANIFEST_CACHE_MAX_ENTRIES: usize = 64;
pub(crate) const MAX_SEARCH_RESPONSE_BYTES: usize = 20 * 1024;
pub(crate) const MAX_TOOL_SCHEMA_ARTIFACTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronHubEntryKind {
    Tool,
    Skill,
}

impl IronHubEntryKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Skill => "skill",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronHubProvenance {
    #[serde(alias = "repo")]
    Official,
    Trusted,
    Verified,
    Private,
    #[default]
    #[serde(alias = "community")]
    New,
}

impl IronHubProvenance {
    pub(crate) fn as_wire(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Trusted => "trusted",
            Self::Verified => "verified",
            Self::Private => "private",
            Self::New => "new",
        }
    }

    pub(crate) fn is_community_unverified(self) -> bool {
        matches!(self, Self::New)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct IronHubManifest {
    pub(crate) version: String,
    pub(crate) generated_at: String,
    pub(crate) release_tag: String,
    pub(crate) repo: String,
    #[serde(default)]
    pub(crate) tools: Vec<IronHubToolEntry>,
    #[serde(default)]
    pub(crate) skills: Vec<IronHubSkillEntry>,
}

impl IronHubManifest {
    pub(crate) fn find_tool(&self, name: &str) -> Option<&IronHubToolEntry> {
        self.tools.iter().find(|entry| entry.name == name)
    }

    pub(crate) fn find_skill(&self, name: &str) -> Option<&IronHubSkillEntry> {
        self.skills.iter().find(|entry| entry.name == name)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct IronHubToolEntry {
    pub(crate) name: String,
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) provenance: IronHubProvenance,
    pub(crate) wasm: IronHubArtifact,
    pub(crate) capabilities: IronHubArtifact,
    /// The extension manifest the registry published for this tool.
    ///
    /// Optional so a catalog predating published manifests still deserializes
    /// and still lists every tool; installing one of those tools is what fails,
    /// with a message naming the cause. Making it required would turn one stale
    /// catalog into zero visible tools.
    #[serde(default)]
    pub(crate) manifest: Option<IronHubArtifact>,
    /// Manifest asset path to digest-pinned schema artifact.
    ///
    /// Kept optional at catalog-deserialization time so stale catalogs remain
    /// searchable. Installation fails closed when the published manifest
    /// references a schema that is absent here.
    #[serde(default)]
    pub(crate) schemas: BTreeMap<String, IronHubArtifact>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct IronHubSkillEntry {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) trunk: String,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) provenance: IronHubProvenance,
    pub(crate) skill_md: IronHubArtifact,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct IronHubArtifact {
    pub(crate) url: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct IronHubInstallOptions {
    pub kind: Option<IronHubEntryKind>,
    pub force: bool,
    pub acknowledge_unverified: bool,
    pub expected_version: Option<String>,
    pub expected_artifact_digest: Option<String>,
    pub private_manifest_url: Option<String>,
}

impl std::fmt::Debug for IronHubInstallOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IronHubInstallOptions")
            .field("kind", &self.kind)
            .field("force", &self.force)
            .field("acknowledge_unverified", &self.acknowledge_unverified)
            .field("expected_version", &self.expected_version)
            .field("expected_artifact_digest", &self.expected_artifact_digest)
            .field(
                "private_manifest_url",
                &self.private_manifest_url.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IronHubCommand {
    Search {
        query: String,
    },
    List {
        kind: Option<IronHubEntryKind>,
    },
    Info {
        name: String,
        kind: Option<IronHubEntryKind>,
    },
    Install {
        name: String,
        options: IronHubInstallOptions,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronHubEntrySummary {
    pub kind: IronHubEntryKind,
    pub name: String,
    pub version: String,
    pub description: String,
    pub provenance: IronHubProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronHubPhase {
    Discovered,
    Installed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronHubResponse {
    pub phase: IronHubPhase,
    /// How many catalog entries MATCHED the request. For a filtered search this
    /// is the size of the match, not the size of the catalog — compare against
    /// `catalog_total` before reporting it as "what is available".
    pub total_entries: usize,
    pub returned_entries: usize,
    pub truncated: bool,
    /// Total entries in the signed catalog, independent of any query filter.
    /// Set on discovery responses so a filtered page cannot be mistaken for the
    /// whole catalog; absent on install responses, which are not a listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_total: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub entries: Vec<IronHubEntrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<LifecycleProductResponse>,
}

impl IronHubResponse {
    pub(crate) fn discovered(entries: Vec<IronHubEntrySummary>) -> Self {
        let total_entries = entries.len();
        Self {
            phase: IronHubPhase::Discovered,
            total_entries,
            returned_entries: total_entries,
            truncated: false,
            catalog_total: None,
            message: None,
            entries,
            lifecycle: None,
        }
    }

    /// `catalog_total` is the size of the whole signed catalog, independent of
    /// any query filter, so a caller can tell a filtered match from the catalog.
    pub(crate) fn discovered_catalog(
        entries: Vec<IronHubEntrySummary>,
        catalog_total: usize,
    ) -> Result<Self, IronHubCommandError> {
        let total_entries = entries.len();
        let mut complete = Self::discovered(entries);
        complete.catalog_total = Some(catalog_total);
        if serialized_len(&complete)? <= MAX_SEARCH_RESPONSE_BYTES {
            return Ok(complete);
        }

        let mut returned_entries = Vec::new();
        let mut serialized_entry_bytes = 0;
        for entry in complete.entries {
            let entry_bytes = serialized_len(&entry)?;
            let candidate_count = returned_entries.len() + 1;
            let base_bytes = serialized_len(&Self::incomplete(
                total_entries,
                candidate_count,
                catalog_total,
                Vec::new(),
            ))?;
            let separator_bytes = candidate_count.saturating_sub(1);
            if base_bytes + serialized_entry_bytes + separator_bytes + entry_bytes
                > MAX_SEARCH_RESPONSE_BYTES
            {
                break;
            }
            serialized_entry_bytes += entry_bytes;
            returned_entries.push(entry);
        }

        Ok(Self::incomplete(
            total_entries,
            returned_entries.len(),
            catalog_total,
            returned_entries,
        ))
    }

    /// `catalog_total` is taken here rather than assigned afterwards so the shape
    /// measured against `MAX_SEARCH_RESPONSE_BYTES` is exactly the shape emitted —
    /// assigning it later made the payload larger than the budget that admitted it.
    fn incomplete(
        total_entries: usize,
        returned_entries: usize,
        catalog_total: usize,
        entries: Vec<IronHubEntrySummary>,
    ) -> Self {
        Self {
            phase: IronHubPhase::Discovered,
            total_entries,
            returned_entries,
            truncated: true,
            catalog_total: Some(catalog_total),
            message: Some(format!(
                "INCOMPLETE IRONHUB RESULTS: returned {returned_entries} of {total_entries} matching catalog entries. Do not claim an unreturned package is absent; narrow the search query or call ironhub_info with its exact name."
            )),
            entries,
            lifecycle: None,
        }
    }
}

fn serialized_len<T: Serialize>(value: &T) -> Result<usize, IronHubCommandError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| IronHubCommandError::Catalog {
            reason: format!("failed to size IronHub catalog response: {error}"),
        })
}

#[derive(Debug, Error)]
pub enum IronHubCommandError {
    #[error("IronHub runtime HTTP egress is unavailable")]
    RuntimeHttpEgressUnavailable,
    #[error("invalid IronHub input: {reason}")]
    InvalidInput { reason: String },
    #[error("IronHub catalog failed: {reason}")]
    Catalog { reason: String },
    #[error("IronHub install failed: {reason}")]
    Install { reason: String },
    #[error("IronHub lifecycle failed: {0}")]
    Product(#[from] ProductOperationFailure),
}

#[derive(Debug, Deserialize)]
pub(crate) struct SignedManifestEnvelope {
    pub(crate) v: u8,
    pub(crate) key_id: String,
    pub(crate) manifest_b64: String,
    pub(crate) sig: String,
}
