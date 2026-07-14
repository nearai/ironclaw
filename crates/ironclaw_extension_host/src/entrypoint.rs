//! Extension entrypoint and the binding rule (overview.md §4.0).
//!
//! Each runtime loader produces one [`ExtensionEntrypoint`] per extension.
//! `bind` is side-effect-free and receives no network/secret/store ports —
//! only the installation context, the resolved contract, and the extension's
//! non-secret config values. It returns the adapters the extension
//! implements; the host then checks them against the resolved contract's
//! declared surfaces (the binding rule) and fails activation on any mismatch.

use std::sync::Arc;

use ironclaw_extensions::ResolvedExtensionManifest;
use ironclaw_host_api::ToolAdapter;
use ironclaw_product_adapters::ChannelAdapter;

/// The bound behavior of one extension: the adapters it implements. Auth
/// never binds (host-managed via recipes); trigger/file are reserved.
#[derive(Clone, Default)]
pub struct ExtensionBindings {
    pub tools: Option<Arc<dyn ToolAdapter>>,
    pub channel: Option<Arc<dyn ChannelAdapter>>,
}

/// Side-effect-free binding context handed to an entrypoint.
pub struct BindContext {
    pub installation_id: String,
    pub resolved: Arc<ResolvedExtensionManifest>,
    /// The extension's non-secret operator config values, keyed by field
    /// handle. Secrets exist only behind host injection and never appear
    /// here.
    pub config: Vec<(String, String)>,
}

/// One extension's loader-produced entrypoint. `bind` must not perform I/O.
pub trait ExtensionEntrypoint: Send + Sync {
    fn bind(&self, ctx: BindContext) -> Result<ExtensionBindings, BindError>;
}

/// Typed binding failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindError {
    /// The manifest declares tools (`[[tools]]`/`[mcp]`) but the entrypoint
    /// bound no tool adapter.
    #[error("extension declares tools but bound no tool adapter")]
    MissingToolAdapter,
    /// The manifest declares a channel but the entrypoint bound no channel
    /// adapter.
    #[error("extension declares a channel but bound no channel adapter")]
    MissingChannelAdapter,
    /// The entrypoint bound a tool adapter the manifest does not declare.
    #[error("extension bound a tool adapter but declares no tools")]
    UndeclaredToolAdapter,
    /// The entrypoint bound a channel adapter the manifest does not declare.
    #[error("extension bound a channel adapter but declares no channel")]
    UndeclaredChannelAdapter,
    /// The loader could not construct the entrypoint.
    #[error("extension could not be loaded: {reason}")]
    Load { reason: String },
}

impl ResolvedExtensionManifestExt for ResolvedExtensionManifest {}

/// Manifest-shape queries the binding rule consumes. A blanket impl on the
/// resolved contract keeps the rule out of the manifest crate.
pub trait ResolvedExtensionManifestExt {
    /// Whether the manifest declares any tools (static `[[tools]]` or an
    /// `[mcp]` server whose discovered tools are model-callable).
    fn declares_tools(&self, resolved: &ResolvedExtensionManifest) -> bool {
        !resolved.tools.is_empty() || resolved.mcp.is_some()
    }

    /// Whether the manifest declares a channel surface.
    fn declares_channel(&self, resolved: &ResolvedExtensionManifest) -> bool {
        resolved.channel.is_some()
    }
}

/// Check bound adapters against the resolved contract: declared surfaces must
/// be bound, and nothing undeclared may be bound (overview §4.0).
pub fn check_binding(
    resolved: &ResolvedExtensionManifest,
    bindings: &ExtensionBindings,
) -> Result<(), BindError> {
    let declares_tools = !resolved.tools.is_empty() || resolved.mcp.is_some();
    let declares_channel = resolved.channel.is_some();

    match (declares_tools, bindings.tools.is_some()) {
        (true, false) => return Err(BindError::MissingToolAdapter),
        (false, true) => return Err(BindError::UndeclaredToolAdapter),
        _ => {}
    }
    match (declares_channel, bindings.channel.is_some()) {
        (true, false) => return Err(BindError::MissingChannelAdapter),
        (false, true) => return Err(BindError::UndeclaredChannelAdapter),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        FakeChannelAdapter, FakeToolAdapter, channel_only_manifest, mcp_manifest,
        tool_and_channel_manifest,
    };

    fn tools_only(tool: bool, channel: bool) -> ExtensionBindings {
        ExtensionBindings {
            tools: tool.then(|| Arc::new(FakeToolAdapter) as Arc<dyn ToolAdapter>),
            channel: channel
                .then(|| Arc::new(FakeChannelAdapter::default()) as Arc<dyn ChannelAdapter>),
        }
    }

    #[test]
    fn declared_tool_without_adapter_fails() {
        let resolved = mcp_manifest();
        let error = check_binding(&resolved, &tools_only(false, false)).unwrap_err();
        assert_eq!(error, BindError::MissingToolAdapter);
    }

    #[test]
    fn declared_channel_without_adapter_fails() {
        let resolved = channel_only_manifest();
        let error = check_binding(&resolved, &tools_only(false, false)).unwrap_err();
        assert_eq!(error, BindError::MissingChannelAdapter);
    }

    #[test]
    fn undeclared_tool_adapter_fails() {
        let resolved = channel_only_manifest();
        let error = check_binding(&resolved, &tools_only(true, true)).unwrap_err();
        assert_eq!(error, BindError::UndeclaredToolAdapter);
    }

    #[test]
    fn undeclared_channel_adapter_fails() {
        let resolved = mcp_manifest();
        let error = check_binding(&resolved, &tools_only(true, true)).unwrap_err();
        assert_eq!(error, BindError::UndeclaredChannelAdapter);
    }

    #[test]
    fn exact_binding_passes() {
        let resolved = tool_and_channel_manifest();
        check_binding(&resolved, &tools_only(true, true)).expect("exact binding");
    }

    #[test]
    fn auth_never_binds_is_not_a_binding_field() {
        // The bindings struct has no auth field — auth is host-managed via
        // recipes and can never be bound. A tool+channel extension that also
        // declares auth still binds cleanly on exactly its two surfaces.
        let resolved = tool_and_channel_manifest();
        assert!(!resolved.auth.is_empty(), "fixture declares auth");
        check_binding(&resolved, &tools_only(true, true)).expect("auth is not a binding");
    }
}
