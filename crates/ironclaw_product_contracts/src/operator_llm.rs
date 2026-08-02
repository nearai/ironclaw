//! Operator LLM-administration vocabulary (PROPOSAL §6.1.3): the provider
//! menu the CLI renders and the request/response bodies the WebUI LLM settings
//! surface serializes. The service ports that produce them (`LlmConfigService`,
//! `ActiveModelReader`) stay with their product-side implementation until the
//! WS5 `operator` row inverts them.
use std::{fmt, path::PathBuf};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderList {
    pub providers: Vec<RebornProviderInfo>,
    #[serde(skip_serializing)]
    pub config_file: PathBuf,
    #[serde(skip_serializing)]
    pub providers_file: PathBuf,
    pub v1_state: RebornV1State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderInfo {
    pub id: String,
    pub description: String,
    pub default_model: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<RebornProviderMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderMetadata {
    pub aliases: Vec<String>,
    pub protocol: String,
    pub model_env: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    pub api_key_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_kind: Option<&'static str>,
    pub accepts_api_key: bool,
    pub can_list_models: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderStatus {
    pub routes: RebornModelRoutesState,
    pub default: Option<RebornProviderSelection>,
    #[serde(skip_serializing)]
    pub config_file: PathBuf,
    #[serde(skip_serializing)]
    pub providers_file: PathBuf,
    pub v1_state: RebornV1State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderSelection {
    pub provider_id: Option<String>,
    pub provider_known: bool,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderWriteOutcome {
    pub provider_id: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub api_key_required: bool,
    pub missing_api_key: bool,
    #[serde(skip_serializing)]
    pub config_file: PathBuf,
    pub v1_state: RebornV1State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedEnvLlm {
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProbeOutcome {
    pub ok: bool,
    pub models: Vec<String>,
    pub message: String,
}

pub const EXAMPLE_OVERLAY_PROVIDER_ID: &str = "example-openrouter";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderMenuEntry {
    pub id: String,
    pub display_name: String,
    pub api_key_required: bool,
    pub description: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RebornV1State {
    #[serde(rename = "not-used")]
    NotUsed,
}

impl RebornV1State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotUsed => "not-used",
        }
    }
}

impl fmt::Display for RebornV1State {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RebornModelRoutesState {
    #[serde(rename = "configured")]
    Configured,
    #[serde(rename = "not-configured")]
    NotConfigured,
}

impl RebornModelRoutesState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::NotConfigured => "not-configured",
        }
    }
}

impl fmt::Display for RebornModelRoutesState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// OAuth identity provider for NEAR AI session login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NearAiAuthProvider {
    Github,
    Google,
}

impl NearAiAuthProvider {
    /// Path segment used in the NEAR AI auth URL (`/v1/auth/<segment>`).
    pub fn as_path(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Google => "google",
        }
    }
}

/// Start a NEAR AI login with the chosen identity provider.
#[derive(Debug, Clone, Deserialize)]
pub struct NearAiLoginRequest {
    pub provider: NearAiAuthProvider,
    /// The browser's own origin (`window.location.origin`), used to build the
    /// NEAR AI `frontend_callback` back to this server's public callback route.
    /// Validated server-side to a bare `scheme://host[:port]`.
    pub origin: String,
}

/// The authorization URL the frontend opens to complete NEAR AI login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearAiLoginStart {
    pub auth_url: String,
}

/// A NEP-413 wallet signature plus the payload it covers, posted by the browser
/// after it connects a NEAR wallet and signs the fixed login message. The server
/// relays this to NEAR AI's `/v1/auth/near` to obtain a session token.
#[derive(Debug, Clone, Deserialize)]
pub struct NearAiWalletLoginRequest {
    pub account_id: String,
    pub public_key: String,
    /// base64-standard encoding of the 64 raw ed25519 signature bytes.
    pub signature: String,
    /// The exact message string the wallet signed.
    pub message: String,
    /// The NEP-413 recipient the wallet signed.
    pub recipient: String,
    /// The 32-byte nonce the wallet signed (first 8 bytes are big-endian epoch
    /// millis).
    pub nonce: Vec<u8>,
    #[serde(default)]
    pub callback_url: Option<String>,
}

/// Result of a completed NEAR AI wallet login. `active` is true once NEAR AI is
/// the live provider; the frontend can then proceed to chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearAiWalletLoginResult {
    pub active: bool,
}

/// The device code + verification URL the frontend displays for Codex login.
/// The user enters `user_code` at `verification_uri`; the backend polls for
/// completion in the background.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexLoginStart {
    pub user_code: String,
    pub verification_uri: String,
}

/// Merged catalog plus the active selection. Keys are masked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmConfigSnapshot {
    pub providers: Vec<LlmProviderView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<LlmActiveSelection>,
}

/// One provider in the merged catalog, annotated for the settings UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmProviderView {
    pub id: String,
    pub description: String,
    /// Protocol/adapter wire name (e.g. `open_ai_completions`, `anthropic`).
    pub adapter: String,
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// `true` for compiled-in providers, `false` for operator-defined ones.
    pub builtin: bool,
    /// Whether this provider is the active selection.
    pub active: bool,
    /// The active model, present only when `active` is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    pub api_key_required: bool,
    /// Whether this provider supports API-key auth at all. This can be true
    /// even when `api_key_required` is false for dual-auth providers.
    pub accepts_api_key: bool,
    /// Whether an API-key value is stored for this provider (never the value).
    pub api_key_set: bool,
    pub can_list_models: bool,
}

/// The active provider + model selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmActiveSelection {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Add or update a custom provider. Deserialize-only (carries a secret).
#[derive(Deserialize)]
pub struct UpsertLlmProviderRequest {
    pub id: String,
    #[serde(default)]
    pub client_action_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// Protocol/adapter wire name.
    pub adapter: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    /// New key value. Absent leaves any stored key untouched; the UI sends the
    /// `••••••••` sentinel for "unchanged" which the impl treats as absent.
    #[serde(default)]
    pub api_key: Option<SecretString>,
    /// When `true`, also make this the active provider.
    #[serde(default)]
    pub set_active: bool,
    /// Model to activate when `set_active` is `true`.
    #[serde(default)]
    pub model: Option<String>,
}

/// Select the active provider + model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetActiveLlmRequest {
    pub provider_id: String,
    #[serde(default)]
    pub model: Option<String>,
}

/// Probe a provider. Deserialize-only (may carry a secret).
#[derive(Deserialize)]
pub struct LlmProbeRequest {
    pub adapter: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub provider_id: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Optional override key for the probe; when absent the impl falls back to
    /// the provider's stored key or env var.
    #[serde(default)]
    pub api_key: Option<SecretString>,
}

/// Result of a connection probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmProbeResult {
    pub ok: bool,
    pub message: String,
}

/// Result of a model-listing probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmModelsResult {
    pub ok: bool,
    #[serde(default)]
    pub models: Vec<String>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `as_str` and `Display` are written twice for each of these states, and
    /// the CLI renders one while the JSON surface serializes the other. Any
    /// pair that drifts reports two different values for the same state.
    #[test]
    fn state_vocabulary_agrees_across_as_str_display_and_serde() {
        let (state, wire) = (RebornV1State::NotUsed, "not-used");
        assert_eq!(state.as_str(), wire);
        assert_eq!(state.to_string(), wire);
        assert_eq!(
            serde_json::to_value(state).expect("serialize"),
            serde_json::json!(wire)
        );

        for (state, wire) in [
            (RebornModelRoutesState::Configured, "configured"),
            (RebornModelRoutesState::NotConfigured, "not-configured"),
        ] {
            assert_eq!(state.as_str(), wire);
            assert_eq!(state.to_string(), wire);
            assert_eq!(
                serde_json::to_value(state).expect("serialize"),
                serde_json::json!(wire)
            );
        }
    }

    /// `as_path` is spliced into the NEAR AI auth URL (`/v1/auth/<segment>`).
    /// A wrong or URL-unsafe segment does not fail here — it fails as a broken
    /// SSO redirect at login time, so the segments are pinned literally.
    #[test]
    fn near_ai_auth_provider_path_segments_are_url_safe_and_match_the_wire_form() {
        for (provider, segment) in [
            (NearAiAuthProvider::Github, "github"),
            (NearAiAuthProvider::Google, "google"),
        ] {
            assert_eq!(provider.as_path(), segment);
            assert_eq!(
                serde_json::to_value(provider).expect("serialize"),
                serde_json::json!(segment),
                "the request body discriminant and the URL segment must not drift"
            );
            assert!(
                segment
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch == '-'),
                "{segment} is not safe to splice into a path unescaped"
            );
        }
    }
}
