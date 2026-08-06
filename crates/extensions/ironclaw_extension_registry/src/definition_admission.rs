use serde::{Deserialize, Serialize};

/// Whether a package definition follows its final installation into removal.
/// Existing definitions preserve the historical remove-with-last-install
/// behavior; tenant-registered catalog definitions opt into retention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageDefinitionRetention {
    #[default]
    RemoveWithLastInstallation,
    RetainInCatalog,
}

/// Result of the single-row immutable package-definition admission CAS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageDefinitionAdmissionOutcome {
    Created,
    ExactExisting,
}
