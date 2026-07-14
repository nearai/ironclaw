//! Loader ports (overview.md §4.0).
//!
//! Each runtime kind produces one [`ExtensionEntrypoint`] per extension. The
//! host does not link the concrete lanes (that would re-couple the layers the
//! architecture gates protect); instead it consults an injected
//! [`ExtensionLoader`] that composition implements as a dispatch over the
//! native factory registry, the WASM tool lane, and the MCP loader. `load`
//! may perform I/O (the MCP loader runs discovery here); the resulting
//! `bind` is side-effect-free.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ResolvedExtensionManifest;

use crate::entrypoint::{BindError, ExtensionEntrypoint};

/// Context handed to a loader when it produces an entrypoint.
pub struct LoadContext {
    pub extension_id: String,
    pub installation_id: String,
    pub resolved: Arc<ResolvedExtensionManifest>,
}

/// A loaded extension: the entrypoint plus, for discovery-owning loaders
/// (hosted MCP), the effective contract the activation publishes.
pub struct LoadedExtension {
    pub entrypoint: Box<dyn ExtensionEntrypoint>,
    /// When present, the activation binds and publishes against this
    /// contract instead of the persisted declaration — the hosted-MCP loader
    /// returns the declared ceiling with the ceiling-validated discovered
    /// tool set folded in, so discovered tools publish atomically with the
    /// generation swap (TOOL-9). The persisted record keeps the declared
    /// contract; the effective contract is never persisted.
    pub effective_resolved: Option<Arc<ResolvedExtensionManifest>>,
}

impl LoadedExtension {
    /// A load with no contract override (static manifests).
    pub fn new(entrypoint: Box<dyn ExtensionEntrypoint>) -> Self {
        Self {
            entrypoint,
            effective_resolved: None,
        }
    }
}

/// Produces a [`LoadedExtension`] for one extension by runtime kind. `load`
/// may perform I/O (the MCP loader runs discovery here); the resulting
/// `bind` is side-effect-free.
#[async_trait]
pub trait ExtensionLoader: Send + Sync {
    async fn load(&self, ctx: &LoadContext) -> Result<LoadedExtension, BindError>;
}

/// One `first_party`-runtime extension implementation the binary assembles
/// (overview.md §4.0): the native loader resolves `runtime.service` against
/// the injected factory set. Composition receives these as input and never
/// links a concrete extension crate.
pub trait NativeExtensionFactory: Send + Sync {
    /// The `runtime.service` identifier this factory serves
    /// (e.g. `some-vendor.extension/v1`).
    fn service(&self) -> &str;

    /// Produce the extension's entrypoint. Runs at load time; `bind` stays
    /// side-effect-free.
    fn load(&self, ctx: &LoadContext) -> Result<Box<dyn ExtensionEntrypoint>, BindError>;
}
