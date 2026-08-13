//! Extension entrypoint and the binding rule (overview.md §4.0).
//!
//! Each runtime loader produces one [`ExtensionEntrypoint`] per extension.
//! `bind` is side-effect-free and receives no network/secret/store ports —
//! only the installation context, the resolved contract, and the extension's
//! non-secret config values. It returns the adapters the extension
//! implements; the host then checks them against the resolved contract's
//! declared surfaces (the binding rule) and fails activation on any mismatch.

use std::sync::Arc;

use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::channel_adapter::ChannelSurfaces;
use ironclaw_extension_contracts::tool_adapter::ToolAdapter;
use ironclaw_extension_registry::{CapabilityVisibility, ResolvedExtensionManifest};

/// The bound behavior of one extension: the adapters it implements. Auth
/// never binds (host-managed via recipes); trigger/file are reserved.
#[derive(Clone, Default)]
pub struct ExtensionBindings {
    pub tools: Option<Arc<dyn ToolAdapter>>,
    /// The channel halves this extension implements. An all-`None` value can
    /// be valid when every declared axis is host-owned (authenticated-session
    /// ingress plus stream reply); [`check_binding`] proves agreement per axis.
    pub channel: ChannelSurfaces,
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
    /// half at all.
    #[error("extension declares a channel but bound no channel adapter")]
    MissingChannelAdapter,
    /// One `[channel.*]` section has no implementing half, or one bound half
    /// has no declaring section. Carries the axis so the operator is told
    /// which declaration to fix rather than that "the channel is wrong".
    #[error("channel {axis} section and adapter half disagree: {detail}")]
    ChannelHalfMismatch {
        axis: &'static str,
        detail: &'static str,
    },
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
    let binds_any_channel_half =
        bindings.channel.ingress.is_some() || bindings.channel.has_outbound();
    if !declares_channel && binds_any_channel_half {
        return Err(BindError::UndeclaredChannelAdapter);
    }
    if let Some(channel) = &resolved.channel {
        check_channel_halves(channel, &bindings.channel)?;
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

/// Each `[channel.*]` section must have exactly the implementing half it
/// declares — this is the check that makes [`ChannelSurfaces`]' three
/// `Option`s worth more than the manifest booleans they replaced. Without it
/// a `None` half is a second copy of a manifest fact with nothing keeping the
/// two in agreement (`.claude/rules/architecture.md` §3); with it, a
/// declaration and its code cannot disagree past activation.
///
/// Two axes are required to be **absent**, and that is the point rather than
/// an exception:
///
/// - `[channel.reply] transport = "stream"` means the host publishes to the
///   durable projection pipeline and the adapter is never called. Binding a
///   reply half there would be dead code that reads as live.
/// - `authenticated_session` ingress is normalized at the host session door,
///   whose actor authority an adapter may never mint. There is no vendor
///   payload to parse, so there is nothing for an ingress half to do.
pub(crate) fn check_channel_halves(
    channel: &ironclaw_extension_contracts::channel::ChannelDescriptor,
    bound: &ChannelSurfaces,
) -> Result<(), BindError> {
    let expected = channel_half_expectations(channel);
    check_half(
        "ingress",
        expected.ingress,
        bound.ingress.is_some(),
        "webhook ingress needs an adapter to parse the vendor payload",
        "authenticated_session ingress is normalized at the host session door",
    )?;

    check_half(
        "reply",
        expected.reply,
        bound.reply.is_some(),
        "a message reply transport needs an adapter to render and send it",
        "a stream reply (or no reply section) is published by the host",
    )?;

    check_half(
        "delivery",
        expected.delivery,
        bound.delivery.is_some(),
        "[channel.delivery] needs an adapter half to send out of band",
        "no [channel.delivery] section declares this channel a delivery target",
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChannelHalfExpectations {
    pub ingress: bool,
    pub reply: bool,
    pub delivery: bool,
}

/// The one declaration-to-half projection used by activation, deployment
/// bindings, and the temporary host-served bridge.
pub(crate) fn channel_half_expectations(
    channel: &ironclaw_extension_contracts::channel::ChannelDescriptor,
) -> ChannelHalfExpectations {
    ChannelHalfExpectations {
        ingress: channel
            .ingress
            .as_ref()
            .is_some_and(|ingress| !ingress.verification.is_authenticated_session()),
        reply: channel.reply_transport() == Some(ReplyTransport::Message),
        delivery: channel.supports_delivery(),
    }
}

fn check_half(
    axis: &'static str,
    declared: bool,
    bound: bool,
    missing_detail: &'static str,
    undeclared_detail: &'static str,
) -> Result<(), BindError> {
    match (declared, bound) {
        (true, false) => Err(BindError::ChannelHalfMismatch {
            axis,
            detail: missing_detail,
        }),
        (false, true) => Err(BindError::ChannelHalfMismatch {
            axis,
            detail: undeclared_detail,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        FakeChannelAdapter, FakeToolAdapter, channel_only_manifest, mcp_manifest,
        session_channel_manifest, tool_and_channel_manifest,
    };

    fn tools_only(tool: bool, channel: bool) -> ExtensionBindings {
        ExtensionBindings {
            tools: tool.then(|| Arc::new(FakeToolAdapter) as Arc<dyn ToolAdapter>),
            channel: if channel {
                FakeChannelAdapter::all_halves()
            } else {
                ChannelSurfaces::default()
            },
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
        assert!(matches!(
            error,
            BindError::ChannelHalfMismatch {
                axis: "ingress",
                ..
            }
        ));
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

    /// The check that makes three `Option`s worth more than the manifest
    /// booleans they replaced. Each arm drops or adds exactly one half against
    /// a manifest that declares the other two, so a failure names the axis.
    #[test]
    fn each_channel_section_must_have_exactly_its_implementing_half() {
        let resolved = channel_only_manifest();
        let adapter = || Arc::new(FakeChannelAdapter::default());

        // Declared but unbound, one axis at a time.
        for (axis, bindings) in [
            (
                "ingress",
                ChannelSurfaces::default()
                    .with_reply(adapter())
                    .with_delivery(adapter()),
            ),
            (
                "reply",
                ChannelSurfaces::default()
                    .with_ingress(adapter())
                    .with_delivery(adapter()),
            ),
            (
                "delivery",
                ChannelSurfaces::default()
                    .with_ingress(adapter())
                    .with_reply(adapter()),
            ),
        ] {
            let error = check_binding(
                &resolved,
                &ExtensionBindings {
                    tools: None,
                    channel: bindings,
                },
            )
            .expect_err("a declared section with no implementing half must fail activation");
            assert!(
                matches!(error, BindError::ChannelHalfMismatch { axis: reported, .. } if reported == axis),
                "expected a {axis} mismatch, got {error:?}"
            );
        }
    }

    /// The two absences that are the point rather than an exception: a
    /// `stream` reply is published by the host, and `authenticated_session`
    /// ingress is normalized at the host session door. Binding a half for
    /// either is dead code that reads as live, so it fails closed.
    #[test]
    fn a_stream_reply_and_session_ingress_must_bind_no_half() {
        let resolved = session_channel_manifest();

        check_binding(
            &resolved,
            &ExtensionBindings {
                tools: None,
                channel: FakeChannelAdapter::delivery_only(),
            },
        )
        .expect("delivery alone is the exact binding for a stream/session channel");

        for (axis, bindings) in [
            (
                "ingress",
                FakeChannelAdapter::delivery_only()
                    .with_ingress(Arc::new(FakeChannelAdapter::default())),
            ),
            (
                "reply",
                FakeChannelAdapter::delivery_only()
                    .with_reply(Arc::new(FakeChannelAdapter::default())),
            ),
        ] {
            let error = check_binding(
                &resolved,
                &ExtensionBindings {
                    tools: None,
                    channel: bindings,
                },
            )
            .expect_err("binding a half the manifest publishes host-side must fail activation");
            assert!(
                matches!(error, BindError::ChannelHalfMismatch { axis: reported, .. } if reported == axis),
                "expected a {axis} mismatch, got {error:?}"
            );
        }
    }

    #[test]
    fn a_fully_host_owned_session_stream_channel_needs_no_adapter_half() {
        let mut resolved = session_channel_manifest();
        resolved.channel.as_mut().expect("session channel").delivery = None;

        check_binding(
            &resolved,
            &ExtensionBindings {
                tools: None,
                channel: ChannelSurfaces::default(),
            },
        )
        .expect("host-owned ingress and reply are a complete operational channel");
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
