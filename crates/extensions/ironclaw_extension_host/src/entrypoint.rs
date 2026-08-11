//! Extension entrypoint and the binding rule (overview.md §4.0).
//!
//! Each runtime loader produces one [`ExtensionEntrypoint`] per extension.
//! `bind` is side-effect-free and receives no network/secret/store ports —
//! only the installation context, the resolved contract, and the extension's
//! non-secret config values. It returns the adapters the extension
//! implements; the host then checks them against the resolved contract's
//! declared surfaces (the binding rule) and fails activation on any mismatch.

use std::sync::Arc;

use ironclaw_extension_contracts::channel_adapter::ChannelAdapter;
use ironclaw_extension_contracts::device_link::DeviceLinkAdapter;
use ironclaw_extension_contracts::linked_session::{
    LinkedAccountResolver, LinkedSessionPortFactory,
};
use ironclaw_extension_contracts::recipe::{DeviceLinkRecipe, VendorAuthRecipe};
use ironclaw_extension_contracts::tool_adapter::ToolAdapter;
use ironclaw_extension_registry::{CapabilityVisibility, ResolvedExtensionManifest};

/// The bound behavior of one extension: the adapters it implements.
///
/// **Auth binds in exactly one shape and no other.** Every recipe-driven
/// method (`oauth2_code`, `api_key`) is data the host engine executes and can
/// never be bound; `device_link` is the single declared exception, because a
/// device-link handshake is a multi-round protocol conversation on a
/// connection the extension owns. That exception is argued in
/// `ADR-device-link-auth-hook.md` (this feature's design record under
/// `docs/internal/design/`), which also records what it costs. Trigger/file
/// remain reserved.
#[derive(Clone, Default)]
pub struct ExtensionBindings {
    pub tools: Option<Arc<dyn ToolAdapter>>,
    pub channel: Option<Arc<dyn ChannelAdapter>>,
    /// The device-link adapter, bound iff the resolved contract declares an
    /// `[auth.<vendor>] method = "device_link"` surface.
    pub device_link: Option<Arc<dyn DeviceLinkAdapter>>,
}

/// Side-effect-free binding context handed to an entrypoint.
pub struct BindContext {
    pub installation_id: String,
    pub resolved: Arc<ResolvedExtensionManifest>,
    /// The extension's non-secret operator config values, keyed by field
    /// handle. Secrets exist only behind host injection and never appear
    /// here.
    pub config: Vec<(String, String)>,
    /// Linked-account session custody for this extension.
    ///
    /// Supplied at bind because custody outlives an invocation: a debounced
    /// write-through, a background runner, and a flush before a pooled client
    /// is dropped all fire outside any `invoke`. Calling
    /// [`LinkedSessionPortFactory::open`] allocates a pre-scoped handle and
    /// performs no I/O, so holding one here keeps `bind` side-effect-free.
    ///
    /// A deployment that wires no custody supplies a fail-closed factory whose
    /// handles report [`ironclaw_extension_contracts::linked_session::LinkedSessionError::Unavailable`]
    /// rather than a `None` every caller has to branch on.
    pub linked_sessions: Arc<dyn LinkedSessionPortFactory>,
    /// Resolves which linked account a tool call acts as.
    ///
    /// Host-implemented for the same reason custody is: the tool ABI carries
    /// no host-issued grant, and an adapter must never mint one from a
    /// caller-supplied id. A deployment that wires no linked-account custody
    /// supplies a fail-closed resolver answering
    /// [`ironclaw_extension_contracts::linked_session::LinkedAccountResolutionError::Unavailable`].
    pub linked_accounts: Arc<dyn LinkedAccountResolver>,
}

/// The device-link recipe this contract declares, if any.
///
/// One reader for the binding rule and one for the driver's host-side mode
/// check, so "does this extension do device-link?" is answered from the
/// resolved recipe in exactly one place.
pub fn declared_device_link_recipe(
    resolved: &ResolvedExtensionManifest,
) -> Option<&DeviceLinkRecipe> {
    resolved
        .auth
        .iter()
        .find_map(|surface| match &surface.recipe {
            Some(VendorAuthRecipe::DeviceLink(recipe)) => Some(recipe.as_ref()),
            _ => None,
        })
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
    /// The manifest declares `[auth.<vendor>] method = "device_link"` but the
    /// entrypoint bound no device-link adapter. Unbound, the method has no
    /// mechanics at all — the recipe is display metadata only — so the
    /// extension would offer a link affordance that can never complete.
    #[error("extension declares a device-link auth surface but bound no device-link adapter")]
    MissingDeviceLinkAdapter,
    /// The entrypoint bound a device-link adapter the manifest does not
    /// declare. Refused for the same reason every other undeclared binding is:
    /// the resolved contract is what the host reviewed, and an adapter outside
    /// it is behavior nobody admitted.
    #[error("extension bound a device-link adapter but declares no device-link auth surface")]
    UndeclaredDeviceLinkAdapter,
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
    let declares_device_link = declared_device_link_recipe(resolved).is_some();

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
    match (declares_device_link, bindings.device_link.is_some()) {
        (true, false) => return Err(BindError::MissingDeviceLinkAdapter),
        (false, true) => return Err(BindError::UndeclaredDeviceLinkAdapter),
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
    // A device-link surface is deliberately absent from this list. Binding it
    // makes auth *mechanics* extension-supplied; it does not make auth a
    // runtime surface. An extension whose only declaration is "the user can
    // link an account" still produces no callable behavior, so it still fails
    // activation here.
    if resolved.tools.is_empty() && !declares_channel && resolved.hooks.is_empty() {
        return Err(BindError::MissingOperationalSurface);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        FakeChannelAdapter, FakeDeviceLinkAdapter, FakeToolAdapter, channel_only_manifest,
        device_link_channel_manifest, mcp_manifest, tool_and_channel_manifest,
    };

    fn bound(tool: bool, channel: bool) -> ExtensionBindings {
        bound_with_device_link(tool, channel, false)
    }

    fn bound_with_device_link(tool: bool, channel: bool, device_link: bool) -> ExtensionBindings {
        ExtensionBindings {
            tools: tool.then(|| Arc::new(FakeToolAdapter) as Arc<dyn ToolAdapter>),
            channel: channel
                .then(|| Arc::new(FakeChannelAdapter::default()) as Arc<dyn ChannelAdapter>),
            device_link: device_link
                .then(|| Arc::new(FakeDeviceLinkAdapter::default()) as Arc<dyn DeviceLinkAdapter>),
        }
    }

    #[test]
    fn declared_tool_without_adapter_fails() {
        let resolved = mcp_manifest();
        let error = check_binding(&resolved, &bound(false, false)).unwrap_err();
        assert_eq!(error, BindError::MissingToolAdapter);
    }

    /// A package resolves its custody handle at bind and keeps it, because
    /// custody outlives an invocation. The factory call is allocation-only,
    /// which is what keeps `bind` side-effect-free.
    #[test]
    fn bind_context_carries_linked_session_custody_that_outlives_bind() {
        use std::sync::Mutex;

        use ironclaw_extension_contracts::linked_session::{
            LinkedAccountGrant, LinkedAccountRef, LinkedSessionPort,
        };
        use ironclaw_host_api::ids::ExtensionId;

        #[derive(Default)]
        struct CustodyProbe {
            held: Mutex<Option<Arc<dyn LinkedSessionPort>>>,
        }

        impl ExtensionEntrypoint for CustodyProbe {
            fn bind(&self, ctx: BindContext) -> Result<ExtensionBindings, BindError> {
                let account = LinkedAccountRef::new("acct-a").map_err(|error| BindError::Load {
                    reason: error.to_string(),
                })?;
                let handle = ctx
                    .linked_sessions
                    .open(&LinkedAccountGrant::new(account, 1));
                if let Ok(mut held) = self.held.lock() {
                    *held = Some(handle);
                }
                Ok(ExtensionBindings::default())
            }
        }

        let probe = CustodyProbe::default();
        probe
            .bind(BindContext {
                installation_id: "acme-link-install".to_string(),
                resolved: Arc::new(device_link_channel_manifest()),
                config: Vec::new(),
                linked_sessions: crate::LinkedSessionStore::unavailable()
                    .custody_for(ExtensionId::new("acme-link").expect("extension id")),
                linked_accounts: Arc::new(
                    crate::linked_session_custody::UnavailableLinkedAccountResolver,
                ),
            })
            .expect("bind resolves custody");

        assert!(
            probe.held.lock().expect("held handle").is_some(),
            "the handle a package took at bind survives the call"
        );
    }

    #[test]
    fn declared_channel_without_adapter_fails() {
        let resolved = channel_only_manifest();
        let error = check_binding(&resolved, &bound(false, false)).unwrap_err();
        assert_eq!(error, BindError::MissingChannelAdapter);
    }

    #[test]
    fn undeclared_tool_adapter_fails() {
        let resolved = channel_only_manifest();
        let error = check_binding(&resolved, &bound(true, true)).unwrap_err();
        assert_eq!(error, BindError::UndeclaredToolAdapter);
    }

    #[test]
    fn undeclared_channel_adapter_fails() {
        let resolved = mcp_manifest();
        let error = check_binding(&resolved, &bound(true, true)).unwrap_err();
        assert_eq!(error, BindError::UndeclaredChannelAdapter);
    }

    #[test]
    fn exact_binding_passes() {
        let resolved = tool_and_channel_manifest();
        check_binding(&resolved, &bound(true, true)).expect("exact binding");
    }

    #[test]
    fn hosted_mcp_template_without_discovered_tools_fails_activation_binding() {
        let resolved = mcp_manifest();
        let error = check_binding(&resolved, &bound(true, false))
            .expect_err("the host-internal MCP connection template is not a usable tool set");
        assert_eq!(error, BindError::EmptyHostedMcpToolCatalog);
    }

    #[test]
    fn channel_only_binding_is_usable_without_model_tools() {
        let resolved = channel_only_manifest();
        check_binding(&resolved, &bound(false, true))
            .expect("a bound channel surface is independently usable");
    }

    #[test]
    fn extension_without_tool_channel_or_hook_surface_fails_activation_binding() {
        let mut resolved = channel_only_manifest();
        resolved.channel = None;
        let error = check_binding(&resolved, &bound(false, false))
            .expect_err("an extension with no operational surface must not activate");
        assert_eq!(error, BindError::MissingOperationalSurface);
    }

    // ─────────────────────────────────────────────────────────────────────
    // RETIRED: `auth_never_binds_is_not_a_binding_field`
    //
    // That test asserted a security invariant — no third-party code executes
    // inside an auth flow, so `ExtensionBindings` has no auth field at all —
    // and the invariant is now deliberately, partially revoked. The decision,
    // its cost, and its compensations are argued in
    // `ADR-device-link-auth-hook.md` — this feature's design record under
    // `docs/internal/design/`, findable with `rg -l ADR-device-link-auth-hook`.
    // That document names this retirement as a consequence and is the record a
    // future reviewer should find. In short: MTProto login is a protocol
    // handshake no recipe descriptor can express, so `device_link` mechanics
    // come from a package-supplied adapter, and the host can no longer claim
    // it verifies the login it displays.
    //
    // What the retirement does NOT license, pinned by the two tests below:
    //
    //   * Every other auth method still binds nothing. `oauth2_code` and
    //     `api_key` remain recipe data the host engine executes, and an
    //     extension declaring one still binds on exactly its tool/channel
    //     surfaces (`recipe_driven_auth_is_still_not_a_binding_field`).
    //   * The hook is not ambient. A device-link adapter bound by an extension
    //     that never declared the surface is refused exactly like any other
    //     undeclared binding
    //     (`undeclared_device_link_adapter_fails`), and a declared surface with
    //     no adapter fails activation rather than shipping a link affordance
    //     that can never complete (`declared_device_link_without_adapter_fails`).
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn recipe_driven_auth_is_still_not_a_binding_field() {
        let resolved = tool_and_channel_manifest();
        assert!(!resolved.auth.is_empty(), "fixture declares auth");
        assert!(
            declared_device_link_recipe(&resolved).is_none(),
            "fixture declares a recipe-driven method, not device-link"
        );
        check_binding(&resolved, &bound(true, true))
            .expect("host-executed auth recipes are not bindings");
        let error = check_binding(&resolved, &bound_with_device_link(true, true, true))
            .expect_err("an oauth2_code surface must not admit a device-link adapter");
        assert_eq!(error, BindError::UndeclaredDeviceLinkAdapter);
    }

    #[test]
    fn declared_device_link_without_adapter_fails() {
        let resolved = device_link_channel_manifest();
        assert!(declared_device_link_recipe(&resolved).is_some());
        let error = check_binding(&resolved, &bound_with_device_link(true, true, false))
            .expect_err("a declared device-link surface has no mechanics until it binds");
        assert_eq!(error, BindError::MissingDeviceLinkAdapter);
    }

    #[test]
    fn undeclared_device_link_adapter_fails() {
        let resolved = channel_only_manifest();
        assert!(declared_device_link_recipe(&resolved).is_none());
        let error = check_binding(&resolved, &bound_with_device_link(false, true, true))
            .expect_err("an undeclared device-link adapter is unreviewed behavior");
        assert_eq!(error, BindError::UndeclaredDeviceLinkAdapter);
    }

    #[test]
    fn declared_device_link_with_adapter_binds() {
        let resolved = device_link_channel_manifest();
        check_binding(&resolved, &bound_with_device_link(true, true, true))
            .expect("a declared and bound device-link surface activates");
    }

    #[test]
    fn device_link_alone_is_not_an_operational_surface() {
        let mut resolved = device_link_channel_manifest();
        resolved.channel = None;
        resolved.tools.clear();
        let error = check_binding(&resolved, &bound_with_device_link(false, false, true))
            .expect_err("linking an account is not, by itself, callable behavior");
        assert_eq!(error, BindError::MissingOperationalSurface);
    }
}
