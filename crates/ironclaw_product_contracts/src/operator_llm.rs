//! The operator LLM-administration port and its wire vocabulary (PROPOSAL
//! §6.1.3, §6.9.2): the provider menu the CLI renders, the request/response
//! bodies the WebUI LLM settings surface serializes, and the service ports that
//! produce them.
//!
//! This is the product-facing contract the WebChat v2 Inference tab consumes to
//! list providers, add/edit/remove custom providers (including an API key), pick
//! the active provider+model, probe a provider (test connection / list models),
//! and drive the NEAR AI / Codex logins. The implementation is
//! `ironclaw_operator::llm_admin::llm_config_service::RebornLlmConfigService`,
//! which owns the provider catalog overlay, the operator-scoped secret store,
//! the config-file writer, and the live provider-reload handle.
//!
//! Declaring [`LlmConfigService`] and [`ActiveModelReader`] here rather than in
//! `ironclaw_product` is what un-inverts the ownership: `ironclaw_operator` is a
//! *sibling* of product, not a consumer of it, so the port it satisfies belongs
//! at the boundary. Product keeps the frozen `llm_config` view descriptor, the
//! "no service wired" fail-closed error, and the `RebornServices` wiring that
//! calls through this port.
//!
//! Wire-safety: inbound API-key values are typed as [`SecretString`] so they
//! never land in `Debug`/logs and are deserialize-only (a request carrying a key
//! can't be serialized back out). Response snapshots never carry a key value —
//! only a boolean `api_key_set`.
use std::{fmt, path::PathBuf};

use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::surface::{ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode};

/// Read-only port exposing the runtime's current active/default model id.
///
/// A WebChat v2 run submitted without an explicit `model` carries no
/// `resolved_model_route`, so its captured `model_usage` has no model id to
/// price against and `RebornGetRunStateResponse::cost` (declared in
/// `ironclaw_product`, so named in prose rather than linked from here) would be
/// `None` even though a real model ran. This port lets the service price such a
/// default-model run against the live provider's active model — which, for a
/// default (unrouted) run, is exactly the model that ran.
///
/// The read must be cheap and synchronous: it is consulted on every run-state
/// poll while a run is in flight. It should reflect operator model hot-swaps
/// (the composition impl reads the live swappable provider handle, not a
/// boot-time snapshot). Returning `None` means "no concrete model to price
/// against" — the run's cost is then omitted rather than mispriced.
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
        caller: ProductSurfaceCaller,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError>;

    /// Add or update a custom provider (and optionally its key / active state).
    async fn upsert_provider(
        &self,
        caller: ProductSurfaceCaller,
        request: UpsertLlmProviderRequest,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError>;

    /// Remove a custom provider and any stored key for it.
    async fn delete_provider(
        &self,
        caller: ProductSurfaceCaller,
        provider_id: String,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError>;

    /// Select the active provider + model.
    async fn set_active(
        &self,
        caller: ProductSurfaceCaller,
        request: SetActiveLlmRequest,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError>;

    /// Probe a provider's credentials/endpoint without persisting anything.
    async fn test_connection(
        &self,
        caller: ProductSurfaceCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmProbeResult, LlmConfigServiceError>;

    /// List the models a provider exposes, without persisting anything.
    async fn list_models(
        &self,
        caller: ProductSurfaceCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmModelsResult, LlmConfigServiceError>;

    /// Begin a NEAR AI browser login (GitHub/Google SSO). Returns the provider
    /// authorization URL for the frontend to open; NEAR AI redirects the browser
    /// back to this server's public callback route, which stores the session
    /// token, makes NEAR AI active, and hot-swaps the running provider. The
    /// caller polls the snapshot until NEAR AI is active.
    async fn start_nearai_login(
        &self,
        caller: ProductSurfaceCaller,
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
        caller: ProductSurfaceCaller,
        request: NearAiWalletLoginRequest,
    ) -> Result<NearAiWalletLoginResult, LlmConfigServiceError>;

    /// Begin an OpenAI Codex (ChatGPT subscription) device-code login. Returns
    /// the user code + verification URL for the frontend to display; a
    /// background task polls the device-auth endpoint, persists the tokens,
    /// makes Codex the active provider, and hot-swaps the running provider. The
    /// caller polls the snapshot until Codex is active.
    async fn start_codex_login(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<CodexLoginStart, LlmConfigServiceError>;
}

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

/// Port-level error surface. The service maps this to the sanitized
/// `ProductSurfaceError` taxonomy; no backend strings, paths, or secrets cross
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

/// The port error's projection onto the transport-visible surface error.
///
/// Defined **once, here** — the same rule WS2.2 established for
/// `ProductOperationFailure`. Both the source and target types are owned by
/// this crate, so a second copy in a consumer could only ever drift: two
/// callers answering different HTTP statuses for one failure is exactly the
/// defect the single-table rule exists to prevent.
///
/// `Unavailable` is the only retryable arm — a filesystem/secret-store/reload
/// backend that is down or unwired may succeed on a retry, while an invalid
/// request, an unknown provider, and a broken invariant will not.
impl From<LlmConfigServiceError> for ProductSurfaceError {
    fn from(error: LlmConfigServiceError) -> Self {
        match error {
            LlmConfigServiceError::InvalidRequest { .. } => ProductSurfaceError::from_status(
                ProductSurfaceErrorCode::InvalidRequest,
                400,
                false,
            ),
            LlmConfigServiceError::NotFound => {
                ProductSurfaceError::from_status(ProductSurfaceErrorCode::NotFound, 404, false)
            }
            LlmConfigServiceError::Unavailable => ProductSurfaceError::service_unavailable(true),
            LlmConfigServiceError::Internal => ProductSurfaceError::internal_invariant(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{TenantId, UserId};
    use secrecy::ExposeSecret as _;
    use std::sync::Arc;

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

    fn caller(user: &str) -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new("tenant").expect("tenant"),
            UserId::new(user).expect("user"),
            None,
            None,
        )
    }

    fn probe(provider_id: &str, key: Option<&str>) -> LlmProbeRequest {
        LlmProbeRequest {
            adapter: "open_ai_completions".to_string(),
            base_url: None,
            provider_id: provider_id.to_string(),
            model: None,
            api_key: key.map(|key| SecretString::from(key.to_string())),
        }
    }

    /// A config double that **threads every argument it is handed** into the
    /// snapshot it returns. A double that discarded `caller` would let the
    /// per-caller assertions below pass against an implementation that served
    /// one operator's provider list to another.
    struct EchoingConfig;

    #[async_trait]
    impl LlmConfigService for EchoingConfig {
        async fn snapshot(
            &self,
            caller: ProductSurfaceCaller,
        ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
            Ok(LlmConfigSnapshot {
                providers: Vec::new(),
                active: Some(LlmActiveSelection {
                    provider_id: caller.user_id.as_str().to_string(),
                    model: None,
                }),
            })
        }

        async fn upsert_provider(
            &self,
            caller: ProductSurfaceCaller,
            request: UpsertLlmProviderRequest,
        ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
            Ok(LlmConfigSnapshot {
                providers: vec![LlmProviderView {
                    id: request.id,
                    description: caller.user_id.as_str().to_string(),
                    adapter: request.adapter,
                    default_model: request.default_model.unwrap_or_default(),
                    base_url: request.base_url,
                    builtin: false,
                    active: request.set_active,
                    active_model: request.model,
                    api_key_required: false,
                    accepts_api_key: true,
                    // The value never crosses back; only whether one arrived.
                    api_key_set: request.api_key.is_some(),
                    can_list_models: false,
                }],
                active: None,
            })
        }

        async fn delete_provider(
            &self,
            _caller: ProductSurfaceCaller,
            provider_id: String,
        ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
            Err(LlmConfigServiceError::InvalidRequest {
                field: Some("provider_id".to_string()),
                reason: provider_id,
            })
        }

        async fn set_active(
            &self,
            _caller: ProductSurfaceCaller,
            request: SetActiveLlmRequest,
        ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
            Ok(LlmConfigSnapshot {
                providers: Vec::new(),
                active: Some(LlmActiveSelection {
                    provider_id: request.provider_id,
                    model: request.model,
                }),
            })
        }

        async fn test_connection(
            &self,
            _caller: ProductSurfaceCaller,
            request: LlmProbeRequest,
        ) -> Result<LlmProbeResult, LlmConfigServiceError> {
            Ok(LlmProbeResult {
                ok: request.api_key.is_some(),
                message: request.provider_id,
            })
        }

        async fn list_models(
            &self,
            _caller: ProductSurfaceCaller,
            request: LlmProbeRequest,
        ) -> Result<LlmModelsResult, LlmConfigServiceError> {
            Ok(LlmModelsResult {
                ok: true,
                models: vec![request.adapter],
                message: request.provider_id,
            })
        }

        async fn start_nearai_login(
            &self,
            _caller: ProductSurfaceCaller,
            request: NearAiLoginRequest,
        ) -> Result<NearAiLoginStart, LlmConfigServiceError> {
            Ok(NearAiLoginStart {
                auth_url: format!("{}/v1/auth/{}", request.origin, request.provider.as_path()),
            })
        }

        async fn complete_nearai_wallet_login(
            &self,
            _caller: ProductSurfaceCaller,
            request: NearAiWalletLoginRequest,
        ) -> Result<NearAiWalletLoginResult, LlmConfigServiceError> {
            Ok(NearAiWalletLoginResult {
                active: !request.account_id.is_empty(),
            })
        }

        async fn start_codex_login(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<CodexLoginStart, LlmConfigServiceError> {
            Ok(CodexLoginStart {
                user_code: "CODE".to_string(),
                verification_uri: "https://example.invalid/device".to_string(),
            })
        }
    }

    /// The port hands the implementation its caller, and the shape admits a
    /// different answer per caller. This pins the contract's plumbing — it does
    /// **not** claim the production service scopes correctly, which is
    /// `ironclaw_operator`'s own test's job.
    #[tokio::test]
    async fn config_port_threads_the_caller_and_can_answer_differently_per_caller() {
        let service: Arc<dyn LlmConfigService> = Arc::new(EchoingConfig);

        let alice = service.snapshot(caller("alice")).await.expect("ok");
        let bob = service.snapshot(caller("bob")).await.expect("ok");

        assert_eq!(
            alice
                .active
                .as_ref()
                .map(|active| active.provider_id.as_str()),
            Some("alice")
        );
        assert_eq!(
            bob.active
                .as_ref()
                .map(|active| active.provider_id.as_str()),
            Some("bob")
        );
        assert_ne!(alice.active, bob.active);
    }

    /// Every field of an upsert request reaches the implementation, and the
    /// key's *presence* — never its value — comes back. This is the wire-safety
    /// property the module doc claims, asserted rather than described.
    #[tokio::test]
    async fn upsert_threads_every_field_and_returns_key_presence_not_the_key() {
        let service: Arc<dyn LlmConfigService> = Arc::new(EchoingConfig);

        let with_key = service
            .upsert_provider(
                caller("alice"),
                UpsertLlmProviderRequest {
                    id: "custom".to_string(),
                    client_action_id: None,
                    name: None,
                    adapter: "anthropic".to_string(),
                    base_url: Some("https://example.invalid".to_string()),
                    default_model: Some("model-a".to_string()),
                    api_key: Some(SecretString::from("super-secret".to_string())),
                    set_active: true,
                    model: Some("model-b".to_string()),
                },
            )
            .await
            .expect("ok");
        let without_key = service
            .upsert_provider(
                caller("alice"),
                UpsertLlmProviderRequest {
                    id: "custom".to_string(),
                    client_action_id: None,
                    name: None,
                    adapter: "anthropic".to_string(),
                    base_url: None,
                    default_model: None,
                    api_key: None,
                    set_active: false,
                    model: None,
                },
            )
            .await
            .expect("ok");

        let view = &with_key.providers[0];
        assert_eq!(view.id, "custom");
        assert_eq!(view.adapter, "anthropic");
        assert_eq!(view.base_url.as_deref(), Some("https://example.invalid"));
        assert_eq!(view.default_model, "model-a");
        assert_eq!(view.active_model.as_deref(), Some("model-b"));
        assert!(view.active);
        assert!(view.api_key_set);

        // Both directions: absence is reported as absence, not as a default.
        assert!(!without_key.providers[0].api_key_set);
        assert!(!without_key.providers[0].active);

        // The response type has no field that could carry a key value at all.
        let rendered = serde_json::to_string(&with_key).expect("serializes");
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("api_key_set"));
    }

    /// A request carrying a secret deserializes it, and the wrapper is
    /// deserialize-only — the type has no `Serialize`, so a handler cannot echo
    /// a submitted key back out even by accident.
    #[test]
    fn probe_request_deserializes_its_secret_and_cannot_be_serialized_back() {
        let request: LlmProbeRequest = serde_json::from_value(serde_json::json!({
            "adapter": "anthropic",
            "provider_id": "custom",
            "api_key": "super-secret",
        }))
        .expect("deserializes");

        assert_eq!(request.provider_id, "custom");
        assert_eq!(
            request
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_string()),
            Some("super-secret".to_string())
        );

        // The direction is the point, and both halves are asserted at compile
        // time rather than described: these request types are *deserialized* on
        // the way in and must never gain a `Serialize` impl, because that is
        // what would let a caller-supplied `SecretString` API key be written
        // back out — to a log line, an echo of the request, or a wire response.
        // `assert_not_impl_any!` is the enforcement; if a future derive adds
        // `Serialize`, these two lines fail the build.
        static_assertions::assert_impl_all!(LlmProbeRequest: serde::de::DeserializeOwned);
        static_assertions::assert_not_impl_any!(LlmProbeRequest: serde::Serialize);
        static_assertions::assert_not_impl_any!(UpsertLlmProviderRequest: serde::Serialize);
    }

    /// Each probe/login argument reaches the implementation distinguishably —
    /// both directions, so a double that returned a constant could not pass.
    #[tokio::test]
    async fn probe_and_login_arguments_reach_the_implementation() {
        let service: Arc<dyn LlmConfigService> = Arc::new(EchoingConfig);

        let keyed = service
            .test_connection(caller("alice"), probe("with-key", Some("k")))
            .await
            .expect("ok");
        let unkeyed = service
            .test_connection(caller("alice"), probe("no-key", None))
            .await
            .expect("ok");
        assert!(keyed.ok);
        assert!(!unkeyed.ok);
        assert_eq!(keyed.message, "with-key");
        assert_eq!(unkeyed.message, "no-key");

        let models = service
            .list_models(caller("alice"), probe("listed", None))
            .await
            .expect("ok");
        assert_eq!(models.models, vec!["open_ai_completions".to_string()]);
        assert_eq!(models.message, "listed");

        let github = service
            .start_nearai_login(
                caller("alice"),
                NearAiLoginRequest {
                    provider: NearAiAuthProvider::Github,
                    origin: "https://app.invalid".to_string(),
                },
            )
            .await
            .expect("ok");
        let google = service
            .start_nearai_login(
                caller("alice"),
                NearAiLoginRequest {
                    provider: NearAiAuthProvider::Google,
                    origin: "https://other.invalid".to_string(),
                },
            )
            .await
            .expect("ok");
        assert_eq!(github.auth_url, "https://app.invalid/v1/auth/github");
        assert_eq!(google.auth_url, "https://other.invalid/v1/auth/google");
        assert_ne!(github.auth_url, google.auth_url);

        let active = service
            .set_active(
                caller("alice"),
                SetActiveLlmRequest {
                    provider_id: "chosen".to_string(),
                    model: Some("chosen-model".to_string()),
                },
            )
            .await
            .expect("ok");
        assert_eq!(
            active.active,
            Some(LlmActiveSelection {
                provider_id: "chosen".to_string(),
                model: Some("chosen-model".to_string()),
            })
        );

        let codex = service
            .start_codex_login(caller("alice"))
            .await
            .expect("ok");
        assert_eq!(codex.user_code, "CODE");

        // The remaining two methods, so the double carries no unexercised arm —
        // an unexercised double method is a contract the suite silently stopped
        // covering. `delete_provider` is also the port's only fallible-by-design
        // path here, and it must carry the caller's argument into the error.
        let deleted = service
            .delete_provider(caller("alice"), "gone".to_string())
            .await
            .expect_err("the double rejects deletes");
        assert!(matches!(
            deleted,
            LlmConfigServiceError::InvalidRequest { ref reason, .. } if reason == "gone"
        ));

        let wallet_ok = service
            .complete_nearai_wallet_login(caller("alice"), wallet_login("alice.near"))
            .await
            .expect("ok");
        let wallet_empty = service
            .complete_nearai_wallet_login(caller("alice"), wallet_login(""))
            .await
            .expect("ok");
        assert!(wallet_ok.active);
        assert!(!wallet_empty.active);
    }

    fn wallet_login(account_id: &str) -> NearAiWalletLoginRequest {
        NearAiWalletLoginRequest {
            account_id: account_id.to_string(),
            public_key: "ed25519:key".to_string(),
            signature: "c2ln".to_string(),
            message: "login".to_string(),
            recipient: "recipient".to_string(),
            nonce: vec![0; 32],
            callback_url: None,
        }
    }

    /// The single status table. Every discriminant maps to exactly one status,
    /// and only the backend-transient arm is retryable — a caller that retried
    /// an invalid request would loop forever against a 400.
    #[test]
    fn port_error_projects_to_one_status_per_discriminant() {
        let cases = [
            (
                LlmConfigServiceError::InvalidRequest {
                    field: Some("provider_id".to_string()),
                    reason: "bad".to_string(),
                },
                ProductSurfaceErrorCode::InvalidRequest,
                400,
                false,
            ),
            (
                LlmConfigServiceError::NotFound,
                ProductSurfaceErrorCode::NotFound,
                404,
                false,
            ),
            (
                LlmConfigServiceError::Unavailable,
                ProductSurfaceErrorCode::Unavailable,
                503,
                true,
            ),
            (
                LlmConfigServiceError::Internal,
                ProductSurfaceErrorCode::Internal,
                500,
                false,
            ),
        ];

        for (error, code, status, retryable) in cases {
            let projected = ProductSurfaceError::from(error);
            assert_eq!(projected.code, code);
            assert_eq!(projected.status_code, status);
            assert_eq!(projected.retryable, retryable);
        }
    }

    /// The `reason` on an invalid request is caller-facing, but the projection
    /// must not carry it onto the wire: a backend path or provider URL in that
    /// string would become a disclosure. The surface error keeps only the
    /// taxonomy.
    #[test]
    fn port_error_projection_drops_the_free_text_reason() {
        let projected = ProductSurfaceError::from(LlmConfigServiceError::InvalidRequest {
            field: Some("base_url".to_string()),
            reason: "/var/secrets/operator/key.pem".to_string(),
        });

        let rendered = serde_json::to_string(&projected).expect("serializes");
        assert!(!rendered.contains("/var/secrets"));
        assert!(!rendered.contains("key.pem"));
        assert_eq!(projected.code, ProductSurfaceErrorCode::InvalidRequest);
    }

    /// Both ports are held as `Arc<dyn _>` by product and composition, so both
    /// must stay object-safe.
    #[test]
    fn llm_config_ports_stay_object_safe() {
        struct FixedReader;
        impl ActiveModelReader for FixedReader {
            fn active_model_id(&self) -> Option<String> {
                Some("model".to_string())
            }
        }

        fn assert_object_safe(_config: &dyn LlmConfigService, reader: &dyn ActiveModelReader) {
            assert_eq!(reader.active_model_id().as_deref(), Some("model"));
        }
        assert_object_safe(&EchoingConfig, &FixedReader);
    }

    /// `None` means "no concrete model to price against" and must stay
    /// distinguishable from a model literally named `"none"` — the run's cost
    /// is omitted in the first case and mispriced in the second.
    #[test]
    fn active_model_reader_distinguishes_absent_from_named() {
        struct Absent;
        impl ActiveModelReader for Absent {
            fn active_model_id(&self) -> Option<String> {
                None
            }
        }
        struct Named;
        impl ActiveModelReader for Named {
            fn active_model_id(&self) -> Option<String> {
                Some("none".to_string())
            }
        }

        assert_eq!(Absent.active_model_id(), None);
        assert_eq!(Named.active_model_id(), Some("none".to_string()));
        assert_ne!(Absent.active_model_id(), Named.active_model_id());
    }

    /// The NEAR AI provider's path segment is what gets pasted into the auth
    /// URL; a rename silently redirects the login.
    #[test]
    fn nearai_auth_provider_path_segments_are_stable() {
        assert_eq!(NearAiAuthProvider::Github.as_path(), "github");
        assert_eq!(NearAiAuthProvider::Google.as_path(), "google");
        assert_eq!(
            serde_json::to_string(&NearAiAuthProvider::Github).expect("serializes"),
            "\"github\""
        );
    }
}
