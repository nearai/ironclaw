//! Public auth facade for `ironclaw_llm`.
//!
//! External callers (the setup wizard, the `ironclaw login` subcommand, and
//! the LLM config loader) interact with provider authentication through this
//! module only. The per-provider implementations (`github_copilot_auth`,
//! `gemini_oauth`, `openai_codex_session`, `codex_auth`) are crate-private —
//! callers must not import them directly.
//!
//! Verbs exposed:
//! - [`start_login`]: run an interactive login flow (device code, OAuth refresh,
//!   etc.) for a [`LoginRequest`].
//! - [`validate_token`]: confirm a manually-supplied token works against the
//!   given backend (used by the wizard's paste-token branches).
//! - [`default_headers`]: provider-specific request headers (e.g. GitHub
//!   Copilot's editor-identity headers) that the LLM config loader merges
//!   into outbound requests.
//! - [`load_persisted_credentials`] / [`default_credentials_path`]: read
//!   credentials another CLI tool (e.g. Codex CLI) has already persisted.

use std::path::{Path, PathBuf};

use secrecy::SecretString;

use crate::codex_auth;
use crate::config::OpenAiCodexConfig;
use crate::github_copilot_auth;
use crate::openai_codex_session::OpenAiCodexSessionManager;

/// Identifies a backend for non-interactive auth queries
/// ([`validate_token`], [`default_headers`]). Mirrors [`LoginRequest`]
/// without the per-backend payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthBackend {
    GithubCopilot,
    Gemini,
    OpenAiCodex,
}

/// Identifies a CLI-style credential file that another tool maintains and
/// `ironclaw_llm` can read directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    /// Codex CLI's `auth.json`. Loaded when the LLM config sees
    /// `LLM_USE_CODEX_AUTH=true`.
    CodexCli,
}

/// Caller-supplied UX hooks for interactive login flows.
///
/// Keeps display/browser-launch concerns out of `ironclaw_llm`: the wizard,
/// the CLI subcommand, and any future TUI/web setup all implement this trait
/// against their own UI layer.
pub trait AuthPrompt: Send + Sync {
    /// Show a device-code verification URL plus the one-time code the user
    /// must enter. The implementation is also responsible for opening the
    /// browser if the host environment supports it.
    fn show_device_code(&self, verification_uri: &str, user_code: &str);
}

/// Per-backend payload for [`start_login`].
///
/// Each variant carries only the inputs the backend actually needs at login
/// time (file paths, endpoint overrides). Constants like client IDs live
/// inside the LLM crate.
#[derive(Debug, Clone)]
pub enum LoginRequest {
    /// GitHub Copilot device-code login. The returned [`AuthOutcome`]
    /// includes the OAuth token in `token_to_persist` so the caller can
    /// store it in their secrets store.
    GithubCopilot,

    /// Gemini Cloud Code OAuth: refresh existing credentials at the given
    /// path or fail. Cloud Code project_id is reported in `display`.
    Gemini { credentials_path: PathBuf },

    /// OpenAI Codex (ChatGPT subscription) device-code login. Tokens are
    /// persisted to disk by `ironclaw_llm`; the caller does not see them.
    OpenAiCodex(OpenAiCodexLoginOptions),
}

/// OpenAI Codex login options, mirroring the `OPENAI_CODEX_*` env vars.
///
/// All fields are optional — `None` means "use the built-in default".
/// Construct from env via [`OpenAiCodexLoginOptions::from_env`].
#[derive(Debug, Clone, Default)]
pub struct OpenAiCodexLoginOptions {
    pub auth_endpoint: Option<String>,
    pub api_base_url: Option<String>,
    pub client_id: Option<String>,
    pub session_path: Option<PathBuf>,
}

impl OpenAiCodexLoginOptions {
    /// Build options by reading `OPENAI_CODEX_AUTH_URL`,
    /// `OPENAI_CODEX_API_URL`, `OPENAI_CODEX_CLIENT_ID`, and
    /// `OPENAI_CODEX_SESSION_PATH` from the environment.
    pub fn from_env() -> Self {
        Self {
            auth_endpoint: std::env::var("OPENAI_CODEX_AUTH_URL").ok(),
            api_base_url: std::env::var("OPENAI_CODEX_API_URL").ok(),
            client_id: std::env::var("OPENAI_CODEX_CLIENT_ID").ok(),
            session_path: std::env::var("OPENAI_CODEX_SESSION_PATH")
                .ok()
                .map(PathBuf::from),
        }
    }

    /// Build options from a fully-resolved [`OpenAiCodexConfig`] — the
    /// shape produced by the binary's `LlmConfig::resolve` pipeline,
    /// which already layered TOML / env / DB precedence.
    ///
    /// Use this from `ironclaw login --openai-codex` so config-file
    /// overrides for endpoints / client id / session path keep working,
    /// not just env vars. Each field becomes `Some(_)` so it wins over
    /// the built-in default in [`Self::into_codex_config`].
    pub fn from_resolved_config(cfg: &OpenAiCodexConfig) -> Self {
        Self {
            auth_endpoint: Some(cfg.auth_endpoint.clone()),
            api_base_url: Some(cfg.api_base_url.clone()),
            client_id: Some(cfg.client_id.clone()),
            session_path: Some(cfg.session_path.clone()),
        }
    }

    fn into_codex_config(self) -> OpenAiCodexConfig {
        let mut cfg = OpenAiCodexConfig::default();
        if let Some(v) = self.auth_endpoint {
            cfg.auth_endpoint = v;
        }
        if let Some(v) = self.api_base_url {
            cfg.api_base_url = v;
        }
        if let Some(v) = self.client_id {
            cfg.client_id = v;
        }
        if let Some(v) = self.session_path {
            cfg.session_path = v;
        }
        cfg
    }
}

/// Outcome of a successful interactive login.
///
/// `token_to_persist` is `Some` when the caller is responsible for storing
/// the credential (e.g. GitHub Copilot — wizard saves to the secrets store).
/// It is `None` when `ironclaw_llm` already persisted credentials on disk
/// (e.g. OpenAI Codex session file).
#[derive(Debug, Default)]
pub struct AuthOutcome {
    pub token_to_persist: Option<SecretString>,
    /// Display-only (key, value) pairs for the caller's setup UX. For
    /// Gemini this includes the Cloud Code project_id when available.
    pub display: Vec<(String, String)>,
}

/// Credentials loaded from a CLI-style auth file (currently Codex CLI's
/// `auth.json`). The caller decides whether to use them as-is or override.
#[derive(Debug)]
pub struct PersistedCredentials {
    pub token: SecretString,
    pub refresh_token: Option<SecretString>,
    /// True when the credential is an OAuth subscription token (e.g. Codex
    /// ChatGPT mode); false for raw API keys. Affects routing/base-URL
    /// decisions on the caller side.
    pub is_subscription: bool,
    /// Provider-specific base URL to honour for this credential type.
    pub base_url: String,
    /// File the credentials were loaded from. Carried forward so the
    /// provider can persist refreshed tokens back to the same place.
    pub source_path: Option<PathBuf>,
}

/// Errors surfaced by the auth facade. Provider-specific error chains are
/// flattened into a single string for the caller — internal types do not
/// cross the boundary.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("{backend}: {reason}")]
    LoginFailed {
        backend: &'static str,
        reason: String,
    },

    #[error("{backend}: token validation failed: {reason}")]
    InvalidToken {
        backend: &'static str,
        reason: String,
    },

    /// The backend does not validate a single bearer token — its
    /// credentials are managed end-to-end via [`start_login`] (OAuth
    /// device-code, credential file, etc.). Callers asking to validate
    /// a plain token against one of these backends should route through
    /// `start_login` instead.
    #[error("{backend:?}: token validation not supported; credentials are managed via start_login")]
    TokenValidationNotSupported { backend: AuthBackend },

    #[error("{0}")]
    Other(String),
}

impl AuthError {
    fn login(backend: &'static str, reason: impl ToString) -> Self {
        Self::LoginFailed {
            backend,
            reason: reason.to_string(),
        }
    }

    fn invalid(backend: &'static str, reason: impl ToString) -> Self {
        Self::InvalidToken {
            backend,
            reason: reason.to_string(),
        }
    }
}

/// Run an interactive login flow.
pub async fn start_login(
    request: LoginRequest,
    prompt: &dyn AuthPrompt,
) -> Result<AuthOutcome, AuthError> {
    match request {
        LoginRequest::GithubCopilot => start_github_copilot_login(prompt).await,
        LoginRequest::Gemini { credentials_path } => start_gemini_login(&credentials_path).await,
        LoginRequest::OpenAiCodex(opts) => start_openai_codex_login(opts).await,
    }
}

/// Validate a manually-supplied token against the backend's auth endpoint.
pub async fn validate_token(backend: AuthBackend, token: &str) -> Result<(), AuthError> {
    match backend {
        AuthBackend::GithubCopilot => {
            let client = http_client()?;
            github_copilot_auth::validate_token(&client, token)
                .await
                .map_err(|e| AuthError::invalid("github_copilot", e))
        }
        AuthBackend::Gemini | AuthBackend::OpenAiCodex => {
            Err(AuthError::TokenValidationNotSupported { backend })
        }
    }
}

/// Default request headers a backend wants on every API call.
pub fn default_headers(backend: AuthBackend) -> Vec<(String, String)> {
    match backend {
        AuthBackend::GithubCopilot => github_copilot_auth::default_headers(),
        AuthBackend::Gemini | AuthBackend::OpenAiCodex => Vec::new(),
    }
}

/// Default file path for a credential source.
pub fn default_credentials_path(source: CredentialSource) -> PathBuf {
    match source {
        CredentialSource::CodexCli => codex_auth::default_codex_auth_path(),
    }
}

/// Load CLI-stored credentials. Returns `None` if the file is missing,
/// unreadable, or contains no usable credentials.
pub fn load_persisted_credentials(
    source: CredentialSource,
    override_path: Option<&Path>,
) -> Option<PersistedCredentials> {
    match source {
        CredentialSource::CodexCli => {
            let path = override_path
                .map(Path::to_path_buf)
                .unwrap_or_else(codex_auth::default_codex_auth_path);
            let creds = codex_auth::load_codex_credentials(&path)?;
            Some(PersistedCredentials {
                base_url: creds.base_url().to_string(),
                token: creds.token,
                refresh_token: creds.refresh_token,
                is_subscription: creds.is_chatgpt_mode,
                source_path: creds.auth_path,
            })
        }
    }
}

// ── Per-backend dispatchers ──────────────────────────────────────────────

fn http_client() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AuthError::Other(format!("http client build failed: {e}")))
}

async fn start_github_copilot_login(prompt: &dyn AuthPrompt) -> Result<AuthOutcome, AuthError> {
    let client = http_client()?;
    let device = github_copilot_auth::request_device_code(&client)
        .await
        .map_err(|e| AuthError::login("github_copilot", e))?;
    prompt.show_device_code(&device.verification_uri, &device.user_code);
    let token = github_copilot_auth::wait_for_device_login(&client, &device)
        .await
        .map_err(|e| AuthError::login("github_copilot", e))?;
    github_copilot_auth::validate_token(&client, &token)
        .await
        .map_err(|e| AuthError::invalid("github_copilot", e))?;
    Ok(AuthOutcome {
        token_to_persist: Some(SecretString::from(token)),
        display: Vec::new(),
    })
}

async fn start_gemini_login(credentials_path: &Path) -> Result<AuthOutcome, AuthError> {
    let manager = crate::gemini_oauth::CredentialManager::new(credentials_path)
        .map_err(|e| AuthError::login("gemini", e))?;
    let cred = manager
        .get_valid_credential()
        .await
        .map_err(|e| AuthError::login("gemini", e))?;
    let mut display = Vec::new();
    if let Some(project_id) = cred.project_id.clone() {
        display.push(("project_id".to_string(), project_id));
    }
    Ok(AuthOutcome {
        token_to_persist: None,
        display,
    })
}

async fn start_openai_codex_login(opts: OpenAiCodexLoginOptions) -> Result<AuthOutcome, AuthError> {
    let mgr = OpenAiCodexSessionManager::new(opts.into_codex_config())
        .map_err(|e| AuthError::login("openai_codex", e))?;
    mgr.device_code_login()
        .await
        .map_err(|e| AuthError::login("openai_codex", e))?;
    Ok(AuthOutcome::default())
}
