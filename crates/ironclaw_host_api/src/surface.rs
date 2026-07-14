//! Capability-surface vocabulary.
//!
//! A *capability surface* is one product-facing face an installed extension
//! declares through its manifest: model-callable tools, an external chat
//! channel, and credential/account acquisition. The surface kind answers
//! "which faces of this extension can be
//! configured and enabled?" — it is product taxonomy.
//!
//! [`crate::RuntimeKind`] is deliberately *not* part of this vocabulary: how
//! an adapter is loaded (`wasm`, `mcp`, `first_party`, ...) never decides
//! whether something is a tool, a channel, or an extension.

use serde::{Deserialize, Serialize};

/// The kind of product-facing surface a manifest declaration projects.
///
/// Extensions declare any combination of these; hosts use the declared kinds
/// for discovery and product grouping instead of maintaining separate
/// first-class product registries beside the extension registry. A kind does
/// not itself authorize or wire runtime services: connection ownership and
/// executable entrypoints require a typed host contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySurfaceKind {
    /// Model/host-callable capability (a tool), e.g. `slack.search_messages`.
    Tool,
    /// External conversation surface: event ingress, verification, identity
    /// binding, and reply egress (e.g. the Slack Events API surface).
    Channel,
    /// Credential/account acquisition the extension's other surfaces depend
    /// on (OAuth accounts, provider tokens).
    Auth,
}

impl CapabilitySurfaceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Channel => "channel",
            Self::Auth => "auth",
        }
    }
}

impl std::fmt::Display for CapabilitySurfaceKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::CapabilitySurfaceKind;

    /// Wire shape is snake_case and round-trips; the string form matches the
    /// serde form so downstream wire fields cannot drift from `as_str()`.
    #[test]
    fn surface_kind_wire_shape_is_snake_case_and_matches_as_str() {
        for (kind, wire) in [
            (CapabilitySurfaceKind::Tool, "\"tool\""),
            (CapabilitySurfaceKind::Channel, "\"channel\""),
            (CapabilitySurfaceKind::Auth, "\"auth\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<CapabilitySurfaceKind>(wire).unwrap(),
                kind
            );
            assert_eq!(format!("\"{kind}\""), wire);
        }
    }
}
