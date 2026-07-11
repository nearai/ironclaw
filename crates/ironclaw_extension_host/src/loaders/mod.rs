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

/// Produces an [`ExtensionEntrypoint`] for one extension by runtime kind.
#[async_trait]
pub trait ExtensionLoader: Send + Sync {
    async fn load(&self, ctx: &LoadContext) -> Result<Box<dyn ExtensionEntrypoint>, BindError>;
}
