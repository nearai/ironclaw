//! LLM configuration port for the WebChat v2 settings surface.
//!
//! This is the product-facing contract the webui2 Inference tab consumes to
//! list providers, add/edit/remove custom providers (including an API key),
//! pick the active provider+model, and probe a provider (test connection /
//! list models). The concrete implementation lives in the composition root
//! (`ironclaw_reborn_composition`), which owns the provider catalog overlay,
//! the operator-scoped secret store, the config-file writer, and the live
//! provider-reload handle. Keeping the port here lets the facade stay the
//! single stable surface the route handlers depend on.
//!
//! Wire-safety: inbound API-key values are typed as [`SecretString`] so they
//! never land in `Debug`/logs and are deserialize-only (a request carrying a
//! key can't be serialized back out). Response snapshots never carry a key
//! value — only a boolean `api_key_set`.

use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::error::{RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind};
use crate::WebUiAuthenticatedCaller;

/// Operator-wide LLM configuration management.
#[async_trait]
pub trait LlmConfigService: Send + Sync {
    /// Current merged catalog + active selection, keys masked.
    async fn snapshot(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError>;

    /// Add or update a custom provider (and optionally its key / active state).
    async fn upsert_provider(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: UpsertLlmProviderRequest,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError>;

    /// Remove a custom provider and any stored key for it.
    async fn delete_provider(
        &self,
        caller: WebUiAuthenticatedCaller,
        provider_id: String,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError>;

    /// Select the active provider + model.
    async fn set_active(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: SetActiveLlmRequest,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError>;

    /// Probe a provider's credentials/endpoint without persisting anything.
    async fn test_connection(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmProbeResult, LlmConfigServiceError>;

    /// List the models a provider exposes, without persisting anything.
    async fn list_models(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmModelsResult, LlmConfigServiceError>;
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
#[derive(Debug, Clone, Deserialize)]
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

/// Port-level error surface. The facade maps this to the sanitized
/// `RebornServicesError` taxonomy; no backend strings, paths, or secrets cross
/// the boundary beyond the user-safe `reason` on `InvalidRequest`.
#[derive(Debug, Clone)]
pub enum LlmConfigServiceError {
    /// Caller-supplied input was invalid. `reason` is user-safe.
    InvalidRequest {
        field: Option<String>,
        reason: String,
    },
    /// The named provider does not exist in the merged catalog.
    NotFound,
    /// The configuration backend (filesystem / secret store / reload) failed
    /// transiently or is not wired.
    Unavailable,
    /// An internal invariant was violated.
    Internal,
}

pub(super) fn map_llm_config_error(error: LlmConfigServiceError) -> RebornServicesError {
    match error {
        LlmConfigServiceError::InvalidRequest { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 400, false)
        }
        LlmConfigServiceError::NotFound => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        LlmConfigServiceError::Unavailable => RebornServicesError::service_unavailable(true),
        LlmConfigServiceError::Internal => RebornServicesError::internal_invariant(),
    }
}

/// Error returned when an LLM-config method is invoked but no service is wired.
pub(super) fn llm_config_unavailable() -> RebornServicesError {
    RebornServicesError::from_status_kind(
        RebornServicesErrorCode::Unavailable,
        RebornServicesErrorKind::ServiceUnavailable,
        503,
        false,
    )
}
