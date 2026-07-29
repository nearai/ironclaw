//! IronHub catalog client for IronClaw Reborn.
//!
//! IronHub is IronClaw's own package registry (`hub.ironclaw.com`): an
//! Ed25519-signed catalog of installable tools and skills. This crate is the
//! host side of that one concrete registry — vendor-scoped by charter, the
//! same way each concrete extension crate is scoped to its product:
//!
//! - **catalog** ([`catalog`], [`model`]): fetch the signed catalog over the
//!   runtime egress port, verify it against deployment-supplied keys, cache it,
//!   and classify entries (provenance tiers, unverified-acknowledgement gates,
//!   pinned private origins).
//! - **install** ([`service`], [`package`]): download digest-verified
//!   artifacts, assemble a package around the registry-published extension
//!   manifest, and drive `ironclaw_extension_host`'s lifecycle manager and the
//!   scoped skill-management port.
//! - **tool surface** ([`capabilities`], [`render`]): the
//!   `builtin.ironhub_search` / `_info` / `_install` model-callable
//!   capabilities.
//! - **deep-link install** ([`agent_link`], [`link_service`]): the
//!   HMAC-shared-key register/deliver flow behind
//!   `ironclaw_product::IronhubLinkService`; link state persists through
//!   `RootFilesystem`.
//!
//! The *generic* registry seam stays in `ironclaw_extension_host`
//! (`registry_extension_package`, `parse_imported_manifest`,
//! `ManifestSource::RegistryInstalled`): a second catalog source would reuse
//! that seam, not this client. Host authority lives here, not in
//! `ironclaw_extension_host` (whose charter excludes egress and shared-key
//! secrets); composition supplies the manifest URL, verify keys, shared key,
//! and runtime ports.

#![warn(unreachable_pub)]

mod agent_link;
mod capabilities;
mod catalog;
mod link_service;
mod model;
mod package;
mod render;
mod service;

#[cfg(test)]
mod tests;

pub use agent_link::{IronhubSharedKey, IronhubSharedKeyError};
pub use capabilities::{
    IRONHUB_INFO_CAPABILITY_ID, IRONHUB_INSTALL_CAPABILITY_ID, IRONHUB_SEARCH_CAPABILITY_ID,
    extend_builtin_first_party_package, insert_handlers,
};
pub use link_service::{
    IronhubLinkBuildError, IronhubLinkStateError, IronhubLinkStateStore, RebornIronhubLinkService,
};
pub use model::{
    IronHubCommand, IronHubCommandError, IronHubEntryKind, IronHubEntrySummary,
    IronHubInstallOptions, IronHubPhase, IronHubProvenance, IronHubResponse,
};
pub use render::render_reborn_ironhub_response;
pub use service::{
    RebornIronHubRuntime, execute_reborn_ironhub_command, execute_reborn_ironhub_service_command,
};
