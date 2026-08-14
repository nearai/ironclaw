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
use ironclaw_extension_registry::ResolvedExtensionManifest;

use crate::entrypoint::{BindError, ExtensionEntrypoint};

/// Context handed to a loader when it produces an entrypoint.
pub struct LoadContext {
    pub extension_id: String,
    pub installation_id: String,
    pub resolved: Arc<ResolvedExtensionManifest>,
    /// Admin-configuration secret material for **this** extension's declared
    /// secret fields, resolvable at load — the one I/O-legal point before
    /// `bind`. Exists for the device-link class of extension, whose vendor
    /// protocol library must hold the operator's application secret
    /// in-process (the declared carve-out); every other factory ignores it.
    /// Pre-scoped to the loading extension: a factory cannot name another
    /// extension's fields through it.
    pub admin_secrets: Arc<dyn LoadTimeAdminSecrets>,
}

/// Load-time access to the loading extension's own secret admin fields.
#[async_trait]
pub trait LoadTimeAdminSecrets: Send + Sync {
    /// The stored value of one of this extension's `secret = true` admin
    /// fields, or `None` when unset (or when the deployment wires no
    /// admin-configuration service). Factories treat `None` as "not
    /// configured" and construct adapters that fail closed.
    async fn secret(
        &self,
        handle: &ironclaw_host_api::ids::SecretHandle,
    ) -> Option<secrecy::SecretString>;
}

/// The fail-closed default: every field reads as unset.
pub struct UnavailableLoadTimeAdminSecrets;

#[async_trait]
impl LoadTimeAdminSecrets for UnavailableLoadTimeAdminSecrets {
    async fn secret(
        &self,
        _handle: &ironclaw_host_api::ids::SecretHandle,
    ) -> Option<secrecy::SecretString> {
        None
    }
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
#[async_trait]
pub trait NativeExtensionFactory: Send + Sync {
    /// The `runtime.service` identifier this factory serves
    /// (e.g. `some-vendor.extension/v1`).
    fn service(&self) -> &str;

    /// Produce the extension's entrypoint. Runs at load time — the one
    /// I/O-legal point (a factory may resolve its extension's admin-secret
    /// fields through [`LoadContext::admin_secrets`]); `bind` stays
    /// side-effect-free.
    async fn load(&self, ctx: &LoadContext) -> Result<Box<dyn ExtensionEntrypoint>, BindError>;
}
