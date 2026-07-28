use std::{fmt, path::PathBuf};

use serde::Serialize;

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
