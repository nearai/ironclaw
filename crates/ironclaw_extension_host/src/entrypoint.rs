//! Extension entrypoint and the binding rule (overview.md §4.0).
//!
//! Each runtime loader produces one [`ExtensionEntrypoint`] per extension.
//! `bind` is side-effect-free and receives no network/secret/store ports —
//! only the installation context, the resolved contract, and the extension's
//! non-secret config values. It returns the adapters the extension
//! implements; the host then checks them against the resolved contract's
//! declared surfaces (the binding rule) and fails activation on any mismatch.

use std::sync::Arc;

use ironclaw_extensions::{CapabilityVisibility, ResolvedExtensionManifest};
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
    /// A hosted-MCP declaration bound only its host-internal connection
    /// template. Activation is not useful until discovery publishes at least
    /// one callable tool from the server's effective contract.
    #[error("hosted MCP discovery published no callable tools")]
    EmptyHostedMcpToolCatalog,
    /// Auth/config metadata alone does not produce runtime behavior. An
    /// activation needs at least one tool, channel, or hook surface.
    #[error("extension declares no tool, channel, or hook surface")]
    MissingOperationalSurface,
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
    if resolved.mcp.is_some()
        && !resolved
            .tools
            .iter()
            .any(|tool| tool.visibility == CapabilityVisibility::Model)
    {
        return Err(BindError::EmptyHostedMcpToolCatalog);
    }
    if resolved.tools.is_empty() && !declares_channel && resolved.hooks.is_empty() {
        return Err(BindError::MissingOperationalSurface);
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
    fn hosted_mcp_template_without_discovered_tools_fails_activation_binding() {
        let resolved = mcp_manifest();
        let error = check_binding(&resolved, &tools_only(true, false))
            .expect_err("the host-internal MCP connection template is not a usable tool set");
        assert_eq!(error, BindError::EmptyHostedMcpToolCatalog);
    }

    #[test]
    fn channel_only_binding_is_usable_without_model_tools() {
        let resolved = channel_only_manifest();
        check_binding(&resolved, &tools_only(false, true))
            .expect("a bound channel surface is independently usable");
    }

    #[test]
    fn extension_without_tool_channel_or_hook_surface_fails_activation_binding() {
        let mut resolved = channel_only_manifest();
        resolved.channel = None;
        let error = check_binding(&resolved, &tools_only(false, false))
            .expect_err("an extension with no operational surface must not activate");
        assert_eq!(error, BindError::MissingOperationalSurface);
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
