//! Capability-surface vocabulary.
//!
//! A *capability surface* is one product-facing face an installed extension
//! declares through its manifest: model-callable tools, an external chat
//! channel, credential/account acquisition, and (reserved) triggers and file
//! exchange. The surface kind answers "which faces of this extension can be
//! configured and enabled?" — it is product taxonomy.
//!
//! [`crate::RuntimeKind`] is deliberately *not* part of this vocabulary: how
//! an adapter is loaded (`wasm`, `mcp`, `first_party`, ...) never decides
//! whether something is a tool, a channel, or an extension.

use serde::{Deserialize, Serialize};

/// The kind of product-facing surface a manifest declaration projects.
///
/// Extensions declare any combination of these; hosts discover and wire
/// generic services from the declared kinds instead of maintaining separate
/// per-kind registries beside the extension registry.
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
    /// External event/schedule trigger surface. Reserved: no manifest section
    /// projects this kind yet.
    Trigger,
    /// File/attachment exchange surface. Reserved: no manifest section
    /// projects this kind yet.
    File,
}

impl CapabilitySurfaceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Channel => "channel",
            Self::Auth => "auth",
            Self::Trigger => "trigger",
            Self::File => "file",
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
            (CapabilitySurfaceKind::Trigger, "\"trigger\""),
            (CapabilitySurfaceKind::File, "\"file\""),
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
