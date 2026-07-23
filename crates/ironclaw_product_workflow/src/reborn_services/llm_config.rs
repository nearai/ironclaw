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
use super::{ProductCapabilityInvoker, RebornServices, RebornViewDescriptor, RebornViewProvider};
use crate::{WebUiAuthenticatedCaller, WebUiInboundValidationCode, WebUiInboundValidationError};

pub const LLM_CONFIG_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "llm_config",
    paginated: false,
};

/// Read-only port exposing the runtime's current active/default model id.
///
/// A WebChat v2 run submitted without an explicit `model` carries no
/// `resolved_model_route`, so its captured `model_usage` has no model id to
/// price against and [`RebornGetRunStateResponse::cost`] would be `None` even
/// though a real model ran. This port lets the facade price such a
/// default-model run against the live provider's active model — which, for a
/// default (unrouted) run, is exactly the model that ran.
///
/// The read must be cheap and synchronous: it is consulted on every run-state
/// poll while a run is in flight. It should reflect operator model hot-swaps
/// (the composition impl reads the live swappable provider handle, not a
/// boot-time snapshot). Returning `None` means "no concrete model to price
/// against" — the run's cost is then omitted rather than mispriced.
///
/// [`RebornGetRunStateResponse::cost`]: crate::RebornGetRunStateResponse::cost
pub trait ActiveModelReader: Send + Sync {
    /// The concrete model id currently backing default (unrouted) runs, or
    /// `None` when no concrete model is configured (cold boot / placeholder) or
    /// the active model is a non-concrete alias.
    fn active_model_id(&self) -> Option<String>;
}

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

    /// Begin a NEAR AI browser login (GitHub/Google SSO). Returns the provider
    /// authorization URL for the frontend to open; NEAR AI redirects the browser
    /// back to this server's public callback route, which stores the session
    /// token, makes NEAR AI active, and hot-swaps the running provider. The
    /// caller polls the snapshot until NEAR AI is active.
    async fn start_nearai_login(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: NearAiLoginRequest,
    ) -> Result<NearAiLoginStart, LlmConfigServiceError>;

    /// Complete a NEAR AI wallet (NEP-413) login. The frontend connects a NEAR
    /// wallet, signs the fixed login message, and posts the signature here; this
    /// exchanges it for a session token at NEAR AI's `/v1/auth/near`, stores the
    /// token, makes NEAR AI active, and hot-swaps the running provider. Unlike
    /// the SSO redirect, wallet signing must happen in the browser, so there is
    /// no server-built auth URL.
    async fn complete_nearai_wallet_login(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: NearAiWalletLoginRequest,
    ) -> Result<NearAiWalletLoginResult, LlmConfigServiceError>;

    /// Begin an OpenAI Codex (ChatGPT subscription) device-code login. Returns
    /// the user code + verification URL for the frontend to display; a
    /// background task polls the device-auth endpoint, persists the tokens,
    /// makes Codex the active provider, and hot-swaps the running provider. The
    /// caller polls the snapshot until Codex is active.
    async fn start_codex_login(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<CodexLoginStart, LlmConfigServiceError>;
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

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    pub(super) async fn invoke_llm_provider_upsert(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: UpsertLlmProviderRequest,
    ) -> Result<(), RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config_unavailable)?;
        super::validate_llm_base_url(request.base_url.as_deref())?;
        service
            .upsert_provider(caller, request)
            .await
            .map_err(map_llm_config_error)?;
        Ok(())
    }

    pub(super) async fn invoke_llm_provider_delete(
        &self,
        caller: WebUiAuthenticatedCaller,
        input: serde_json::Value,
    ) -> Result<(), RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config_unavailable)?;
        let provider_id = input
            .as_object()
            .and_then(|object| object.get("provider_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| llm_config_input_error("provider_id"))?
            .to_string();
        service
            .delete_provider(caller, provider_id)
            .await
            .map_err(map_llm_config_error)?;
        Ok(())
    }

    pub(super) async fn invoke_llm_active_set(
        &self,
        caller: WebUiAuthenticatedCaller,
        input: serde_json::Value,
    ) -> Result<(), RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config_unavailable)?;
        let request: SetActiveLlmRequest =
            serde_json::from_value(input).map_err(|_| llm_config_input_error("input"))?;
        service
            .set_active(caller, request)
            .await
            .map_err(map_llm_config_error)?;
        Ok(())
    }

    pub(super) async fn build_llm_config_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config_unavailable)?;
        service.snapshot(caller).await.map_err(map_llm_config_error)
    }
}

fn llm_config_input_error(field: &'static str) -> RebornServicesError {
    RebornServicesError::from(WebUiInboundValidationError::new(
        field,
        WebUiInboundValidationCode::InvalidValue,
    ))
}
