//! Extension registry: metadata catalog for tools and channels.
//!
//! The registry provides a central index of all available extensions (WASM tools
//! and channels) with their source locations, build artifacts, authentication
//! requirements, and grouping via bundles.
//!
//! ```text
//! registry/
//! ├── tools/          <- One JSON manifest per tool
//! ├── channels/       <- One JSON manifest per channel
//! └── _bundles.json   <- Bundle definitions (google, messaging, default)
//! ```

pub mod artifacts;
pub mod catalog;
pub mod embedded;
pub mod hub_installer;
pub mod hub_manifest;
pub mod installer;
pub mod manifest;

pub use catalog::{RegistryCatalog, RegistryError};
pub use hub_installer::{HubInstallOutcome, HubInstaller};
pub use hub_manifest::{
    DEFAULT_HUB_MANIFEST_URL, HubArtifact, HubManifest, HubSkillEntry, HubToolEntry, Provenance,
};
pub use installer::RegistryInstaller;
pub use manifest::{
    ArtifactSpec, AuthSummary, BundleDefinition, BundlesFile, ExtensionManifest, ManifestKind,
    SourceSpec,
};
