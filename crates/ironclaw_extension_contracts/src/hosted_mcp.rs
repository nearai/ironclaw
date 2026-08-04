//! Neutral wire vocabulary for user-registered hosted MCP servers.
//!
//! These types intentionally describe only untrusted registration input. The
//! extension host owns endpoint canonicalization, SSRF checks, identity
//! derivation, authentication contracts, and durable admission.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use ironclaw_host_api::error::HostApiError;
use ironclaw_host_api::ids::ExtensionId;

use crate::lifecycle_id::LifecyclePackageId;

/// A bounded, redacted location advertised by a hosted MCP server while
/// challenging an unauthenticated request.
///
/// This deliberately carries a location only: response text, credentials, and
/// arbitrary authentication parameters must not cross the runtime boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct McpAuthMetadataLocation(String);

impl McpAuthMetadataLocation {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        const MAX_BYTES: usize = 2_048;
        let value = value.into();
        let value = value.trim();
        if value.is_empty()
            || value.len() > MAX_BYTES
            || value.chars().any(|character| character.is_control())
            || !(value.starts_with("https://") || value.starts_with("http://"))
        {
            return Err(HostApiError::invalid_id(
                "mcp_auth_metadata_location",
                value,
                "MCP auth metadata location must be a bounded HTTP(S) location without control characters",
            ));
        }

        // Metadata locations identify a document; query and fragment text are
        // not part of that identity and can carry bearer material.
        let end = value.find(['?', '#']).unwrap_or(value.len());
        let normalized = value[..end].trim_end_matches('/');
        if normalized.is_empty() {
            return Err(HostApiError::invalid_id(
                "mcp_auth_metadata_location",
                value,
                "MCP auth metadata location must not be empty after normalization",
            ));
        }
        Ok(Self(normalized.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for McpAuthMetadataLocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Maximum number of metadata locations extracted from a single untrusted
/// `WWW-Authenticate`/`Protected-Resource-Metadata` header value. A hosted MCP
/// server response is attacker-controlled input, and both consumers
/// (`ironclaw_host_runtime::egress::sanitize` and `ironclaw_mcp`) `extend()`
/// unconditionally from this helper's output, so the ceiling must live here
/// rather than at either call site. A real challenge header carries at most a
/// couple of `resource`/`resource_metadata` params plus the whole-header
/// fallback; 8 leaves generous headroom while bounding fan-out from a header
/// crafted with many repeated params.
const MAX_MCP_AUTH_METADATA_LOCATIONS: usize = 8;

/// Extract the only OAuth challenge values permitted to cross the runtime
/// response-sanitization boundary. Raw challenge parameters and response text
/// remain discarded; query and fragment data are removed by
/// [`McpAuthMetadataLocation::new`]. Bounded to
/// [`MAX_MCP_AUTH_METADATA_LOCATIONS`] entries regardless of how many
/// candidate parts an attacker-controlled header contains.
pub fn extract_mcp_auth_metadata_locations(value: &str) -> Vec<McpAuthMetadataLocation> {
    value
        .split([' ', ','])
        .filter_map(|part| {
            part.strip_prefix("resource_metadata=")
                .or_else(|| part.strip_prefix("resource="))
        })
        .map(|part| part.trim_matches('"'))
        .chain(
            std::iter::once(value)
                .filter(|value| value.starts_with("https://") || value.starts_with("http://")),
        )
        .filter_map(|value| McpAuthMetadataLocation::new(value).ok())
        .take(MAX_MCP_AUTH_METADATA_LOCATIONS)
        .collect()
}

/// Redacted protocol output for a hosted-MCP 401/403 response.
///
/// It is intentionally limited to the status and metadata document locations
/// extracted from authentication response headers. In particular it never
/// retains the remote response body, challenge text, or token values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAuthChallenge {
    pub status: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub www_authenticate_metadata: Vec<McpAuthMetadataLocation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_resource_metadata: Vec<McpAuthMetadataLocation>,
}

/// Maximum size of a hosted MCP endpoint supplied at an external boundary.
pub const HOSTED_MCP_ENDPOINT_MAX_BYTES: usize = 2_048;

/// Map a user-friendly registration id into the namespace reserved for
/// user-registered hosted MCP packages.
pub fn hosted_mcp_extension_id(
    desired_id: &LifecyclePackageId,
) -> Result<ExtensionId, HostApiError> {
    if desired_id.as_str().starts_with("mcp-") {
        return Err(HostApiError::invalid_id(
            "hosted_mcp_extension_id",
            desired_id.as_str(),
            "desired id must not include the reserved mcp- prefix",
        ));
    }
    ExtensionId::new(format!("mcp-{}", desired_id.as_str()))
}

/// Opaque, bounded endpoint input. Admission validates and canonicalizes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedMcpEndpoint(String);

impl HostedMcpEndpoint {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(HostApiError::invalid_id(
                "hosted_mcp_endpoint",
                value,
                "hosted MCP endpoint must not be empty",
            ));
        }
        if value.len() > HOSTED_MCP_ENDPOINT_MAX_BYTES {
            return Err(HostApiError::invalid_id(
                "hosted_mcp_endpoint",
                value,
                format!(
                    "hosted MCP endpoint must be at most {HOSTED_MCP_ENDPOINT_MAX_BYTES} bytes"
                ),
            ));
        }
        if trimmed
            .chars()
            .any(|character| character == '\0' || character.is_control())
        {
            return Err(HostApiError::invalid_id(
                "hosted_mcp_endpoint",
                value,
                "hosted MCP endpoint must not contain NUL/control characters",
            ));
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for HostedMcpEndpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HostedMcpEndpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Caller-selected hosted MCP authentication shape. Credentials never cross
/// this request boundary.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostedMcpAuthSelection {
    /// Probe without credentials during registration. Resolve no-auth or OAuth
    /// only when the protocol proves it; an otherwise unexplained 401 requires
    /// the caller to retry registration with an explicit OAuth or bearer choice.
    Auto,
    #[default]
    NoAuth,
    Bearer,
    #[serde(rename = "oauth")]
    OAuth {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_profile_id: Option<String>,
    },
}

/// Untrusted request to create or join a caller-owned hosted MCP package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterHostedMcpRequest {
    pub desired_id: LifecyclePackageId,
    pub desired_name: String,
    pub endpoint: HostedMcpEndpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_selection: Option<HostedMcpAuthSelection>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_wire_omits_auth_when_not_supplied() {
        let request = RegisterHostedMcpRequest {
            desired_id: LifecyclePackageId::new("calendar").expect("valid id"),
            desired_name: "Calendar".to_string(),
            endpoint: HostedMcpEndpoint::new("https://mcp.example.test").expect("endpoint"),
            auth_selection: None,
        };

        assert_eq!(
            serde_json::to_value(request).expect("serialize"),
            serde_json::json!({
                "desired_id": "calendar",
                "desired_name": "Calendar",
                "endpoint": "https://mcp.example.test"
            })
        );
    }

    #[test]
    fn desired_id_maps_once_into_reserved_namespace() {
        assert_eq!(
            hosted_mcp_extension_id(&LifecyclePackageId::new("linear").expect("id"))
                .expect("mapped id")
                .as_str(),
            "mcp-linear"
        );
        assert!(
            hosted_mcp_extension_id(&LifecyclePackageId::new("mcp-linear").expect("id")).is_err()
        );
    }

    #[test]
    fn hosted_mcp_endpoint_deserialization_rejects_malformed_values() {
        for value in [
            serde_json::json!(""),
            serde_json::json!("https://mcp.example.test/\npath"),
            serde_json::json!("x".repeat(HOSTED_MCP_ENDPOINT_MAX_BYTES + 1)),
            serde_json::json!(42),
        ] {
            assert!(
                serde_json::from_value::<HostedMcpEndpoint>(value).is_err(),
                "malformed hosted MCP endpoint must be rejected"
            );
        }
    }

    #[test]
    fn direct_remote_source_serializes_with_exact_endpoint() {
        let source = ironclaw_host_api::trust::PackageSource::DirectRemote {
            endpoint: "https://mcp.example.test/rpc?tenant=one".to_string(),
        };
        let json = serde_json::to_value(&source).expect("serialize source");
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "direct_remote",
                "endpoint": "https://mcp.example.test/rpc?tenant=one"
            })
        );
        assert_eq!(
            serde_json::from_value::<ironclaw_host_api::trust::PackageSource>(json)
                .expect("deserialize source"),
            source
        );
    }

    #[test]
    fn auth_selection_wire_shapes_are_closed_and_exact() {
        assert_eq!(
            serde_json::to_value(HostedMcpAuthSelection::Auto).expect("serialize"),
            serde_json::json!({ "kind": "auto" })
        );
        assert_eq!(
            serde_json::to_value(HostedMcpAuthSelection::NoAuth).expect("serialize"),
            serde_json::json!({ "kind": "no_auth" })
        );
        assert_eq!(
            serde_json::to_value(HostedMcpAuthSelection::Bearer).expect("serialize"),
            serde_json::json!({ "kind": "bearer" })
        );
        assert_eq!(
            serde_json::to_value(HostedMcpAuthSelection::OAuth {
                client_profile_id: Some("github-default".to_string()),
            })
            .expect("serialize"),
            serde_json::json!({
                "kind": "oauth",
                "client_profile_id": "github-default"
            })
        );
    }

    #[test]
    fn auth_challenge_metadata_locations_strip_token_bearing_url_parts() {
        let location = McpAuthMetadataLocation::new(
            " https://issuer.example.test/.well-known/oauth-protected-resource?token=secret#fragment ",
        )
        .expect("bounded HTTPS metadata location");

        assert_eq!(
            location.as_str(),
            "https://issuer.example.test/.well-known/oauth-protected-resource"
        );
        assert!(McpAuthMetadataLocation::new("not a location").is_err());
    }

    #[test]
    fn auth_metadata_location_deserialize_routes_through_validation() {
        let rejected: Result<McpAuthMetadataLocation, _> =
            serde_json::from_value(serde_json::json!("not a location"));
        assert!(rejected.is_err(), "non-HTTP(S) value must be rejected");

        let oversized = "https://issuer.example.test/".to_string() + &"a".repeat(3_000);
        let rejected: Result<McpAuthMetadataLocation, _> =
            serde_json::from_value(serde_json::json!(oversized));
        assert!(rejected.is_err(), "oversized value must be rejected");

        let accepted: McpAuthMetadataLocation =
            serde_json::from_value(serde_json::json!("https://issuer.example.test/metadata"))
                .expect("valid HTTPS location deserializes");
        assert_eq!(accepted.as_str(), "https://issuer.example.test/metadata");
    }

    #[test]
    fn auth_metadata_locations_are_capped_regardless_of_header_fan_out() {
        let many_resources = (0..64)
            .map(|index| format!("resource=\"https://issuer.example.test/r{index}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let locations = extract_mcp_auth_metadata_locations(&many_resources);
        assert_eq!(locations.len(), MAX_MCP_AUTH_METADATA_LOCATIONS);
    }

    #[test]
    fn notion_style_auth_challenge_extracts_only_metadata_location() {
        let locations = extract_mcp_auth_metadata_locations(
            "Bearer realm=\"OAuth\", resource_metadata=\"https://mcp.notion.com/.well-known/oauth-protected-resource/mcp\", error=\"invalid_token\", error_description=\"Missing or invalid access token\"",
        );
        assert_eq!(locations.len(), 1);
        assert_eq!(
            locations[0].as_str(),
            "https://mcp.notion.com/.well-known/oauth-protected-resource/mcp"
        );
    }

    #[test]
    fn stripe_style_unquoted_auth_challenge_extracts_metadata_location() {
        let locations = extract_mcp_auth_metadata_locations(
            "Bearer resource_metadata=https://mcp.stripe.com/.well-known/oauth-protected-resource",
        );
        assert_eq!(locations.len(), 1);
        assert_eq!(
            locations[0].as_str(),
            "https://mcp.stripe.com/.well-known/oauth-protected-resource"
        );
    }
}
