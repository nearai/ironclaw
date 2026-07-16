//! Wire types for the Trace Commons onboarding endpoint.
//! Contract: TraceCommons/trace-commons-server#137. Field names must match exactly.

use serde::{Deserialize, Serialize};

pub const ONBOARD_REQUEST_SCHEMA_VERSION: &str = "trace_commons.onboard_request.v1";
pub const ONBOARD_RESPONSE_SCHEMA_VERSION: &str = "trace_commons.onboard_response.v1";

#[derive(Debug, Clone, Serialize)]
pub struct OnboardRequest {
    pub schema_version: &'static str,
    pub invite_code: String,
    /// base64 (standard, padded) of the raw Ed25519 public key bytes.
    pub device_public_key: String,
    pub client_info: OnboardClientInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnboardClientInfo {
    pub agent: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OnboardResponse {
    pub schema_version: String,
    pub tenant_id: String,
    pub ingest_url: String,
    pub issuer_url: String,
    pub audience: String,
    pub device_key_id: String,
    #[serde(default)]
    pub contributor_label: Option<String>,
    /// Optional browser-surface navigation hints (trace-commons-server#137).
    /// Deployment config, NOT credential material: these never participate in
    /// issuer trust anchoring; non-HTTPS values are dropped, not fatal.
    #[serde(default)]
    pub community_url: Option<String>,
    #[serde(default)]
    pub profile_url: Option<String>,
    #[serde(default)]
    pub leaderboard_url: Option<String>,
}

/// Typed error codes from the onboard endpoint. `InviteNotValid` deliberately
/// covers unknown/consumed/revoked — the server keeps those indistinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardErrorCode {
    InviteNotValid,
    InviteMalformed,
    DeviceKeyMalformed,
    OnboardRateLimited,
    Unknown,
}

impl OnboardErrorCode {
    pub fn parse(code: &str) -> Self {
        match code {
            "InviteNotValid" => Self::InviteNotValid,
            "InviteMalformed" => Self::InviteMalformed,
            "DeviceKeyMalformed" => Self::DeviceKeyMalformed,
            "OnboardRateLimited" => Self::OnboardRateLimited,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboard_request_serializes_to_contract_shape() {
        let req = OnboardRequest {
            schema_version: ONBOARD_REQUEST_SCHEMA_VERSION,
            invite_code: "INV9K3RT5FBQ72JX".to_string(),
            device_public_key: "AAAA".to_string(),
            client_info: OnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.1.0".to_string(),
            },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["schema_version"], "trace_commons.onboard_request.v1");
        assert_eq!(json["invite_code"], "INV9K3RT5FBQ72JX");
        assert_eq!(json["device_public_key"], "AAAA");
        assert_eq!(json["client_info"]["agent"], "ironclaw");
    }

    #[test]
    fn onboard_response_round_trips_with_optional_label() {
        let json = serde_json::json!({
            "schema_version": "trace_commons.onboard_response.v1",
            "tenant_id": "tenant-zaki-pilot",
            "ingest_url": "https://ingest.example.com",
            "issuer_url": "https://issuer.example.com",
            "audience": "trace-commons-ingest",
            "device_key_id": "sha256:abc123",
        });
        let resp: OnboardResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.tenant_id, "tenant-zaki-pilot");
        assert!(resp.contributor_label.is_none());
        assert!(resp.profile_url.is_none()); // community URLs are optional
    }

    #[test]
    fn onboard_response_parses_community_urls_when_present() {
        let json = serde_json::json!({
            "schema_version": "trace_commons.onboard_response.v1",
            "tenant_id": "t", "ingest_url": "https://i.example",
            "issuer_url": "https://s.example", "audience": "a",
            "device_key_id": "sha256:x",
            "community_url": "https://tracecommons.ai",
            "profile_url": "https://tracecommons.ai/profile",
            "leaderboard_url": "https://tracecommons.ai/leaderboard",
        });
        let resp: OnboardResponse = serde_json::from_value(json).unwrap();
        assert_eq!(
            resp.profile_url.as_deref(),
            Some("https://tracecommons.ai/profile")
        );
    }

    #[test]
    fn onboard_error_code_parses_known_and_unknown() {
        assert_eq!(
            OnboardErrorCode::parse("InviteNotValid"),
            OnboardErrorCode::InviteNotValid
        );
        assert_eq!(
            OnboardErrorCode::parse("SomethingNew"),
            OnboardErrorCode::Unknown
        );
    }
}
