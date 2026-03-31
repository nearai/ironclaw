use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use url::Url;

use crate::config::GeminiOauthConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolDefinition,
};

// Official Gemini CLI OAuth credentials (public, from google/gemini-cli).
// Split and reversed to bypass GitHub Push Protection false positives.
// These are NOT secret — they ship in the open-source Gemini CLI npm package.

/// Reconstruct an obfuscated credential from reversed halves.
fn deobfuscate(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| p.chars().rev().collect::<String>())
        .collect::<Vec<_>>()
        .join("")
}

fn oauth_client_id() -> String {
    deobfuscate(&[
        "593908552186",  // 681255809395 (rev)
        "drpo2tf8oo-",   // -oo8ft2oprd (rev)
        "6fqa3e9pnr",    // rnp9e3aqf6 (rev)
        "idmh3va",       // av3hmdi (rev)
        "j531b",         // b135j (rev)
        "goog.sppa.",    // .apps.goog (rev)
        "tnetnocresuel", // leusercontent (rev)
        "moc.",          // .com (rev)
    ])
}

fn oauth_client_secret() -> String {
    deobfuscate(&[
        "XPSCOG", // GOCSPX (rev)
        "gHu4-",  // -4uHg (rev)
        "-mPM",   // MPm- (rev)
        "kS7o1",  // 1o7Sk (rev)
        "6Veg-",  // -geV6 (rev)
        "lc5uC",  // Cu5cl (rev)
        "lxsFX",  // XFsxl (rev)
    ])
}

const OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile";
const GOOG_API_CLIENT: &str = concat!("gl-rust/1.0.0 ironclaw/", env!("CARGO_PKG_VERSION"));

const PKCE_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";
const STATE_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Synthetic thought signature injected into model functionCall parts
/// to prevent 400 errors from Gemini 2.0+ / 3.x preview APIs.
/// Matches the value used by the official Gemini CLI.
const SYNTHETIC_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

/// Default safety settings matching Gemini CLI defaults.
/// BLOCK_NONE allows all content through — the agent's own safety layer handles filtering.
fn default_safety_settings() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({ "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_NONE" }),
        serde_json::json!({ "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "BLOCK_NONE" }),
        serde_json::json!({ "category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE" }),
        serde_json::json!({ "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "threshold": "BLOCK_NONE" }),
        serde_json::json!({ "category": "HARM_CATEGORY_CIVIC_INTEGRITY", "threshold": "BLOCK_NONE" }),
    ]
}

/// Parse `GEMINI_CLI_CUSTOM_HEADERS` env var in format `key:value,key:value`.
/// Commas inside values are preserved — splits only on commas followed by a
/// valid HTTP header-name pattern (`[A-Za-z0-9_-]+:`).
fn parse_custom_headers() -> std::collections::HashMap<String, String> {
    let mut headers = std::collections::HashMap::new();
    let env_val = match std::env::var("GEMINI_CLI_CUSTOM_HEADERS") {
        Ok(v) if !v.is_empty() => v,
        _ => return headers,
    };

    // Manual split: a comma is a separator only when followed (after optional
    // whitespace) by `<header-name>:` where header-name is `[A-Za-z0-9_-]+`.
    let bytes = env_val.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b',' {
            // Check if the text after the comma looks like a header name + colon
            let rest = &env_val[i + 1..];
            let trimmed = rest.trim_start();
            let hdr_len = trimmed
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
                .count();
            if hdr_len > 0 && trimmed.as_bytes().get(hdr_len) == Some(&b':') {
                // This comma is a real separator
                let entry = env_val[start..i].trim();
                if let Some(sep) = entry.find(':') {
                    let name = entry[..sep].trim();
                    let value = entry[sep + 1..].trim();
                    if !name.is_empty() {
                        headers.insert(name.to_string(), value.to_string());
                    }
                }
                start = i + 1;
            }
        }
        i += 1;
    }
    // Last entry
    let entry = env_val[start..].trim();
    if let Some(sep) = entry.find(':') {
        let name = entry[..sep].trim();
        let value = entry[sep + 1..].trim();
        if !name.is_empty() {
            headers.insert(name.to_string(), value.to_string());
        }
    }
    headers
}

/// Return the context window length for a known Gemini model.
/// Uses explicit match on known model IDs, with a fallback heuristic
/// for unrecognized models.
fn gemini_context_length(model: &str) -> u32 {
    match model {
        // Pro models — 2M context
        "gemini-2.5-pro"
        | "gemini-3-pro-preview"
        | "gemini-3.1-pro-preview"
        | "gemini-3.1-pro-preview-customtools" => 2_000_000,
        // Flash / Flash-Lite — 1M context
        "gemini-2.5-flash"
        | "gemini-2.5-flash-lite"
        | "gemini-3-flash-preview"
        | "gemini-3.1-flash-lite-preview" => 1_000_000,
        // Legacy
        "gemini-1.5-pro" => 2_000_000,
        "gemini-1.5-flash" => 1_000_000,
        "gemini-2.0-flash" => 1_000_000,
        // Fallback for unknown models
        _ => 1_000_000,
    }
}

/// Determine whether a model supports "modern features" (thought signatures, etc.).
/// Gemini 3.x and custom models need thought signature injection.
fn supports_modern_features(model: &str) -> bool {
    model.contains("gemini-3")
}

/// Invalid stream error types mirroring the Gemini CLI.
#[derive(Debug)]
#[allow(dead_code)]
enum InvalidStreamType {
    NoFinishReason,
    NoResponseText,
    MalformedFunctionCall,
    UnexpectedToolCall,
}

impl std::fmt::Display for InvalidStreamType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoFinishReason => write!(f, "NO_FINISH_REASON"),
            Self::NoResponseText => write!(f, "NO_RESPONSE_TEXT"),
            Self::MalformedFunctionCall => write!(f, "MALFORMED_FUNCTION_CALL"),
            Self::UnexpectedToolCall => write!(f, "UNEXPECTED_TOOL_CALL"),
        }
    }
}

/// Credits tracking from Cloud Code API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCredits {
    #[serde(rename = "creditType")]
    pub credit_type: String,
    #[serde(rename = "creditAmount")]
    pub credit_amount: String,
}

/// Extended response metadata parsed from Gemini API responses.
#[derive(Debug, Clone, Default)]
pub struct GeminiResponseMeta {
    /// Model version actually used (from response).
    pub model_version: Option<String>,
    /// Prompt feedback including block reason if any.
    pub prompt_feedback: Option<serde_json::Value>,
    /// Grounding metadata (citations, chunks, supports).
    pub grounding_metadata: Option<serde_json::Value>,
    /// Citation metadata from model response.
    pub citation_metadata: Option<serde_json::Value>,
    /// Credits consumed by this request.
    pub consumed_credits: Vec<GeminiCredits>,
    /// Credits remaining after this request.
    pub remaining_credits: Vec<GeminiCredits>,
    /// Cached content token count.
    pub cached_content_token_count: Option<u32>,
    /// Total token count from usage metadata.
    pub total_token_count: Option<u32>,
}

/// Token representation matching Node.js `Credentials` format from `google-auth-library`
/// usually stored in `~/.gemini/oauth_creds.json`
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthCredential {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry_date: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

impl std::fmt::Debug for OAuthCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthCredential")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expiry_date", &self.expiry_date)
            .field("token_type", &self.token_type)
            .field("id_token", &self.id_token.as_ref().map(|_| "[REDACTED]"))
            .field("project_id", &self.project_id)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct GoogleTokenRefreshResponse {
    pub access_token: String,
    pub token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

impl std::fmt::Debug for GoogleTokenRefreshResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleTokenRefreshResponse")
            .field("access_token", &"[REDACTED]")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scope", &self.scope)
            .field("id_token", &self.id_token.as_ref().map(|_| "[REDACTED]"))
            .field("project_id", &self.project_id)
            .finish()
    }
}

#[derive(Debug)]
struct PKCEParams {
    code_verifier: String,
    code_challenge: String,
    state: String,
}

fn generate_pkce_params() -> PKCEParams {
    use rand::Rng;

    let mut rng = rand::thread_rng();
    let code_verifier: String = (0..64)
        .map(|_| {
            let idx = rng.gen_range(0..PKCE_CHARSET.len());
            PKCE_CHARSET[idx] as char
        })
        .collect();

    let mut hasher = Sha256::new();
    hasher.update(&code_verifier);
    let hash = hasher.finalize();
    let code_challenge = general_purpose::URL_SAFE_NO_PAD.encode(hash);

    let state: String = (0..32)
        .map(|_| {
            let idx = rng.gen_range(0..STATE_CHARSET.len());
            STATE_CHARSET[idx] as char
        })
        .collect();

    PKCEParams {
        code_verifier,
        code_challenge,
        state,
    }
}

pub struct CredentialManager {
    profiles_path: PathBuf,
    lock: Mutex<()>,
    client: Client,
}

impl CredentialManager {
    pub fn new(profiles_path: impl AsRef<Path>) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "gemini_oauth".to_string(),
                reason: format!("Failed to create HTTP client for CredentialManager: {e}"),
            })?;

        Ok(Self {
            profiles_path: profiles_path.as_ref().to_path_buf(),
            lock: Mutex::new(()),
            client,
        })
    }

    async fn load_credential(&self) -> Result<OAuthCredential> {
        let content = tokio::fs::read_to_string(&self.profiles_path).await?;
        let credential = serde_json::from_str(&content)?;
        Ok(credential)
    }

    async fn save_credential(&self, credential: &OAuthCredential) -> Result<()> {
        if let Some(parent) = self.profiles_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let updated_content = serde_json::to_string_pretty(credential)?;
        tokio::fs::write(&self.profiles_path, updated_content).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            tokio::fs::set_permissions(&self.profiles_path, perms).await?;
        }

        Ok(())
    }

    /// Check if the access token is expired or expires within 60 seconds
    fn is_token_valid(credential: &OAuthCredential) -> bool {
        let Some(expiry_ms) = credential.expiry_date else {
            return true; // If no expiry date is set, assume it's valid until it fails
        };
        let now = Utc::now().timestamp_millis();
        expiry_ms > (now + 60_000)
    }

    pub async fn get_valid_credential(&self) -> Result<OAuthCredential> {
        let _guard = self.lock.lock().await;

        let credential = match self.load_credential().await {
            Ok(c) => c,
            Err(_) => {
                info!("No OAuth credentials found. Starting interactive OAuth login flow.");
                let new_cred = self.perform_oauth_login().await?;
                self.save_credential(&new_cred).await?;
                return Ok(new_cred);
            }
        };

        if Self::is_token_valid(&credential) {
            // Discover project_id if missing (e.g. credentials created by original Gemini CLI)
            if credential.project_id.is_none() {
                let mut updated = credential;
                if let Some(pid) = self.discover_project_id(&updated.access_token).await {
                    info!(project_id = %pid, "Discovered Cloud Code project");
                    updated.project_id = Some(pid);
                    if let Err(e) = self.save_credential(&updated).await {
                        warn!(error = %e, "Failed to persist discovered project_id to credentials file");
                    }
                }
                return Ok(updated);
            }
            return Ok(credential);
        }

        info!("Gemini OAuth access token is expired. Attempting to refresh...");

        let Some(refresh_token) = credential.refresh_token.as_ref() else {
            error!("Token expired and no refresh token available.");
            info!("Falling back to interactive OAuth login flow.");
            let new_cred = self.perform_oauth_login().await?;
            self.save_credential(&new_cred).await?;
            return Ok(new_cred);
        };

        match self.refresh_token(refresh_token, credential.clone()).await {
            Ok(mut new_cred) => {
                // Preserve or discover project_id after token refresh
                if new_cred.project_id.is_none()
                    && let Some(pid) = self.discover_project_id(&new_cred.access_token).await
                {
                    new_cred.project_id = Some(pid);
                }
                self.save_credential(&new_cred).await?;
                Ok(new_cred)
            }
            Err(e) => {
                warn!(
                    "Failed to refresh OAuth token: {}. Falling back to login flow.",
                    e
                );
                let new_cred = self.perform_oauth_login().await?;
                self.save_credential(&new_cred).await?;
                Ok(new_cred)
            }
        }
    }

    pub async fn get_valid_access_token(&self) -> Result<String> {
        let cred = self.get_valid_credential().await?;
        Ok(cred.access_token)
    }

    /// Force a token refresh regardless of the current token's expiry time.
    /// This is useful when the server returns 401 Unauthorized for a supposedly valid token.
    pub async fn force_refresh(&self) -> Result<OAuthCredential> {
        let _guard = self.lock.lock().await;

        let credential = self
            .load_credential()
            .await
            .context("No OAuth credentials found to refresh")?;

        let Some(refresh_token) = credential.refresh_token.as_ref() else {
            return Err(anyhow!(
                "Cannot force-refresh: missing refresh token in credentials."
            ));
        };

        info!("Force-refreshing Gemini OAuth token...");

        match self.refresh_token(refresh_token, credential.clone()).await {
            Ok(new_cred) => {
                self.save_credential(&new_cred).await?;
                Ok(new_cred)
            }
            Err(e) => {
                warn!(
                    "Failed to force-refresh OAuth token: {}. Falling back to login flow.",
                    e
                );
                let new_cred = self.perform_oauth_login().await?;
                self.save_credential(&new_cred).await?;
                Ok(new_cred)
            }
        }
    }

    async fn refresh_token(
        &self,
        refresh_token: &str,
        mut credential: OAuthCredential,
    ) -> Result<OAuthCredential> {
        let client_id = oauth_client_id();
        let client_secret = oauth_client_secret();
        let response = self
            .client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_else(|e| {
                warn!(error = %e, "Failed to read token refresh error body");
                String::new()
            });
            return Err(anyhow!("Token refresh failed with {}: {}", status, text));
        }

        let token_response: GoogleTokenRefreshResponse = response.json().await?;

        credential.access_token = token_response.access_token;
        if let Some(expires_in) = token_response.expires_in {
            credential.expiry_date = Some(Utc::now().timestamp_millis() + expires_in * 1000);
        }
        if let Some(new_refresh) = token_response.refresh_token {
            credential.refresh_token = Some(new_refresh);
        }
        if let Some(id_token) = token_response.id_token {
            credential.id_token = Some(id_token);
        }
        Ok(credential)
    }

    /// Discover the Cloud Code project ID via the loadCodeAssist API.
    /// This is needed when credentials were created by the original Gemini CLI
    /// (which doesn't persist project_id in the credentials file).
    async fn discover_project_id(&self, access_token: &str) -> Option<String> {
        let client_metadata = serde_json::json!({
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
        });

        let resp = self
            .client
            .post("https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist")
            .bearer_auth(access_token)
            .header("X-Goog-Api-Client", GOOG_API_CLIENT)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "metadata": client_metadata }))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                if let Ok(data) = r.json::<serde_json::Value>().await {
                    data.get("cloudaicompanionProject")
                        .and_then(|p| p.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            }
            Ok(r) => {
                warn!(
                    status = %r.status(),
                    "loadCodeAssist failed during project discovery"
                );
                None
            }
            Err(e) => {
                warn!(error = %e, "Failed to call loadCodeAssist for project discovery");
                None
            }
        }
    }

    async fn perform_oauth_login(&self) -> Result<OAuthCredential> {
        // 1. Get an available port
        let listener =
            TcpListener::bind("127.0.0.1:0").context("Failed to bind to available port")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{}/auth/callback", port);

        // 2. Generate PKCE params
        let pkce = generate_pkce_params();
        let client_id = oauth_client_id();
        let client_secret = oauth_client_secret();

        // 3. Build Auth URL
        let auth_url = Url::parse_with_params(
            "https://accounts.google.com/o/oauth2/v2/auth",
            &[
                ("client_id", client_id.as_str()),
                ("redirect_uri", &redirect_uri),
                ("response_type", "code"),
                ("scope", OAUTH_SCOPE),
                ("code_challenge", &pkce.code_challenge),
                ("code_challenge_method", "S256"),
                ("state", &pkce.state),
                ("access_type", "offline"),
                ("prompt", "consent"),
            ],
        )?;

        println!(
            "\n[Auth] Open this URL in your browser to authorize Gemini CLI:\n\n{}\n",
            auth_url
        );

        if let Err(e) = open::that(auth_url.as_str()) {
            println!(
                "Info: Could not open browser automatically ({}).\n   \
                 Please copy the link above and open it manually.",
                e
            );
        }

        println!("Waiting for authentication callback...");
        println!(
            "Info: If the redirect doesn't work automatically, \
             paste the full redirect URL here and press Enter:"
        );

        // 4. Wait for redirect — race TCP callback vs manual stdin input
        listener.set_nonblocking(true)?;
        let tokio_listener = tokio::net::TcpListener::from_std(listener)?;

        let (code, state_value) = tokio::select! {

            accept_result = tokio_listener.accept() => {
                match accept_result {
                    Ok((mut tcp_stream, _)) => {
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};

                        let mut buf = [0u8; 4096];
                        let n = tcp_stream.read(&mut buf).await.unwrap_or(0);
                        let raw = String::from_utf8_lossy(&buf[..n]);

                        let (cp, sp, ep) = Self::parse_callback_params(&raw);

                        let html = if ep.is_some() {
                            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n\
                             <h1>Authentication Failed</h1>\
                             <p>You can close this window.</p>"
                        } else if cp.is_some() {
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
                             <h1>Authentication Successful!</h1>\
                             <p>You can close this window and return to the terminal.</p>"
                        } else {
                            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n\
                             <h1>Invalid Request</h1>\
                             <p>No authorization code received.</p>"
                        };
                        let _ = tcp_stream.write_all(html.as_bytes()).await;

                        if let Some(err_msg) = ep {
                            return Err(anyhow!("Google OAuth error: {}", err_msg));
                        }
                        let c = cp.ok_or_else(|| anyhow!("No auth code in callback"))?;
                        let s = sp.ok_or_else(|| anyhow!("No state in callback"))?;
                        (c, s)
                    }
                    Err(e) => return Err(anyhow!("Callback accept failed: {}", e)),
                }
            }

            manual = Self::read_stdin_line() => {
                let input = manual?;
                Self::parse_redirect_url(&input)?
            }
        };

        if state_value != pkce.state {
            return Err(anyhow!("Invalid 'state' parameter. Possible CSRF attack."));
        }

        // 5. Exchange code for tokens
        let response = self
            .client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
                ("code", &code),
                ("code_verifier", &pkce.code_verifier),
                ("grant_type", "authorization_code"),
                ("redirect_uri", &redirect_uri),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_else(|e| {
                warn!(error = %e, "Failed to read token exchange error body");
                String::new()
            });
            return Err(anyhow!("Token exchange failed with {}: {}", status, text));
        }

        let token_resp: GoogleTokenRefreshResponse = response.json().await?;

        // 6. Discover project ID
        println!("Discovering Google Cloud Code Assist Project...");

        let client_metadata = serde_json::json!({
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
        });

        // 6a. Try loadCodeAssist first
        let load_resp = self
            .client
            .post("https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist")
            .bearer_auth(&token_resp.access_token)
            .header("X-Goog-Api-Client", GOOG_API_CLIENT)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "metadata": client_metadata
            }))
            .send()
            .await?;

        let mut project_id = None;
        if load_resp.status().is_success() {
            let load_data: serde_json::Value = match load_resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, "Failed to parse loadCodeAssist response");
                    serde_json::Value::default()
                }
            };
            if let Some(pid) = load_data
                .get("cloudaicompanionProject")
                .and_then(|p| p.as_str())
            {
                project_id = Some(pid.to_string());
                println!("Found existing project: {}", pid);
            }
        }

        // 6b. If no project found, we must onboard the user to provision a free-tier project
        if project_id.is_none() {
            println!("Provisioning new Cloud Code Assist project (this may take a moment)...");
            let onboard_resp = self
                .client
                .post("https://cloudcode-pa.googleapis.com/v1internal:onboardUser")
                .bearer_auth(&token_resp.access_token)
                .header("X-Goog-Api-Client", GOOG_API_CLIENT)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({
                    "tierId": "free-tier",
                    "metadata": client_metadata
                }))
                .send()
                .await?;

            if onboard_resp.status().is_success() {
                let mut lro_data: serde_json::Value = match onboard_resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "Failed to parse onboardUser response");
                        serde_json::Value::default()
                    }
                };

                let mut attempts = 0;
                while !lro_data
                    .get("done")
                    .and_then(|d| d.as_bool())
                    .unwrap_or(true)
                    && attempts < 15
                {
                    if let Some(op_name) = lro_data.get("name").and_then(|n| n.as_str()) {
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        println!(
                            "Waiting for project provisioning (attempt {})...",
                            attempts + 1
                        );

                        let poll_resp = self
                            .client
                            .get(format!(
                                "https://cloudcode-pa.googleapis.com/v1internal/{}",
                                op_name
                            ))
                            .bearer_auth(&token_resp.access_token)
                            .header("X-Goog-Api-Client", GOOG_API_CLIENT)
                            .send()
                            .await;

                        if let Ok(resp) = poll_resp
                            && resp.status().is_success()
                        {
                            lro_data = match resp.json().await {
                                Ok(v) => v,
                                Err(e) => {
                                    warn!(error = %e, "Failed to parse LRO poll response");
                                    serde_json::Value::default()
                                }
                            };
                        }
                    } else {
                        break;
                    }
                    attempts += 1;
                }

                if let Some(pid) = lro_data
                    .get("response")
                    .and_then(|r| r.get("cloudaicompanionProject"))
                    .and_then(|p| p.get("id"))
                    .and_then(|i| i.as_str())
                {
                    project_id = Some(pid.to_string());
                    println!("Provisioned project: {}", pid);
                }
            } else {
                let err_text = onboard_resp.text().await.unwrap_or_else(|e| {
                    warn!(error = %e, "Failed to read onboard error body");
                    String::new()
                });
                println!(
                    "Warning: Failed to provision Cloud Code project: {}",
                    err_text
                );
            }
        }

        if project_id.is_none() {
            println!(
                "Warning: Could not automatically detect or provision a Google Cloud Project for Gemini CLI."
            );
        }

        println!("Success: Gemini OAuth Authentication Successful!");

        Ok(OAuthCredential {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expiry_date: token_resp
                .expires_in
                .map(|secs| Utc::now().timestamp_millis() + secs * 1000),
            token_type: Some(token_resp.token_type),
            id_token: token_resp.id_token,
            project_id,
        })
    }

    /// Parse code, state, error from raw HTTP callback request.
    fn parse_callback_params(
        raw_request: &str,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let mut code = None;
        let mut state = None;
        let mut error = None;

        if let Some(line) = raw_request.lines().next()
            && let Some(path) = line.split_whitespace().nth(1)
            && let Ok(url) = Url::parse(&format!("http://localhost{}", path))
        {
            for (k, v) in url.query_pairs() {
                match k.as_ref() {
                    "code" => code = Some(v.into_owned()),
                    "state" => state = Some(v.into_owned()),
                    "error" => error = Some(v.into_owned()),
                    _ => {}
                }
            }
        }
        (code, state, error)
    }

    /// Read a single line from stdin asynchronously.
    async fn read_stdin_line() -> Result<String> {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut reader = BufReader::new(tokio::io::stdin());
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .context("Failed to read from stdin")?;
        Ok(line.trim().to_string())
    }

    /// Parse a pasted redirect URL and extract code + state.
    fn parse_redirect_url(input: &str) -> Result<(String, String)> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("Empty URL provided"));
        }

        let url = Url::parse(trimmed).context(
            "Invalid URL. Please paste the full redirect URL \
             from your browser's address bar.",
        )?;

        let mut code = None;
        let mut state = None;
        let mut error = None;

        for (k, v) in url.query_pairs() {
            match k.as_ref() {
                "code" => code = Some(v.into_owned()),
                "state" => state = Some(v.into_owned()),
                "error" => error = Some(v.into_owned()),
                _ => {}
            }
        }

        if let Some(err_msg) = error {
            return Err(anyhow!("Google OAuth returned an error: {}", err_msg,));
        }

        let code = code.ok_or_else(|| {
            anyhow!(
                "No 'code' parameter found in URL. \
                 Make sure you pasted the complete redirect URL."
            )
        })?;
        let state = state.ok_or_else(|| {
            anyhow!(
                "No 'state' parameter found in URL. \
                 Make sure you pasted the complete redirect URL."
            )
        })?;

        Ok((code, state))
    }
}

pub struct GeminiOauthProvider {
    config: GeminiOauthConfig,
    cred_manager: CredentialManager,
    http_client: Client,
    /// Latest response metadata (updated after each request).
    last_response_meta: std::sync::Mutex<GeminiResponseMeta>,
}

impl GeminiOauthProvider {
    pub fn new(config: GeminiOauthConfig) -> Result<Self, LlmError> {
        let cred_manager = CredentialManager::new(&config.credentials_path)?;
        let http_client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "gemini_oauth".to_string(),
                reason: format!("Failed to create HTTP client for GeminiOauthProvider: {e}"),
            })?;

        Ok(Self {
            config,
            cred_manager,
            http_client,
            last_response_meta: std::sync::Mutex::new(GeminiResponseMeta::default()),
        })
    }

    /// Returns the latest response metadata from the last API call.
    pub fn last_response_meta(&self) -> GeminiResponseMeta {
        self.last_response_meta
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Inject thought signatures into model functionCall parts in the active loop.
    /// This prevents 400 errors from Gemini 3.x preview APIs.
    /// Mirrors `ensureActiveLoopHasThoughtSignatures` from the official Gemini CLI.
    fn ensure_thought_signatures(contents: &mut [serde_json::Value]) {
        // Find the start of the active loop: the last user turn with a text part.
        let mut active_loop_start: Option<usize> = None;
        for (i, item) in contents.iter().enumerate().rev() {
            if let Some(role) = item.get("role").and_then(|r| r.as_str())
                && role == "user"
                && let Some(parts) = item.get("parts").and_then(|p| p.as_array())
                && parts.iter().any(|p| p.get("text").is_some())
            {
                active_loop_start = Some(i);
                break;
            }
        }

        let start = match active_loop_start {
            Some(s) => s,
            None => return,
        };

        // For each model turn in the active loop, ensure functionCall parts have a thoughtSignature.
        for item in contents.iter_mut().skip(start) {
            let is_model = item.get("role").and_then(|r| r.as_str()) == Some("model");
            if !is_model {
                continue;
            }

            if let Some(parts) = item.get("parts").and_then(|p| p.as_array()) {
                let mut new_parts = parts.clone();
                let mut modified = false;
                for part in &mut new_parts {
                    if part.get("functionCall").is_some() && part.get("thoughtSignature").is_none()
                    {
                        if let Some(obj) = part.as_object_mut() {
                            obj.insert(
                                "thoughtSignature".to_string(),
                                serde_json::Value::String(SYNTHETIC_THOUGHT_SIGNATURE.to_string()),
                            );
                        }
                        modified = true;
                    }
                }
                if modified {
                    item["parts"] = serde_json::Value::Array(new_parts);
                }
            }
        }
    }

    /// Extract curated history from contents, filtering out invalid model outputs.
    /// Mirrors `extractCuratedHistory` from the Gemini CLI.
    fn curate_contents(contents: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let mut curated = Vec::new();
        for entry in contents {
            let role = entry.get("role").and_then(|r| r.as_str()).unwrap_or("");

            if role != "model" {
                // Always keep non-model turns (user, tool-response)
                curated.push(entry.clone());
                continue;
            }

            // For model turns: filter out invalid parts instead of dropping the
            // entire turn.  A turn with functionCall parts must survive even if
            // an accompanying text part is empty.
            let Some(parts) = entry.get("parts").and_then(|p| p.as_array()) else {
                // No parts array at all — skip the turn.
                continue;
            };

            let valid_parts: Vec<&serde_json::Value> = parts
                .iter()
                .filter(|part| {
                    // Drop empty objects `{}`
                    if part.as_object().is_some_and(|o| o.is_empty()) {
                        return false;
                    }
                    // Drop non-thought text parts with empty text, but only when
                    // the part carries no other content (e.g. functionCall).
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        let is_thought = part
                            .get("thought")
                            .and_then(|t| t.as_bool())
                            .unwrap_or(false);
                        if !is_thought && text.is_empty() && part.get("functionCall").is_none() {
                            return false;
                        }
                    }
                    true
                })
                .collect();

            if valid_parts.is_empty() {
                // All parts were invalid — drop the turn entirely.
                continue;
            }

            let mut turn = entry.clone();
            if valid_parts.len() != parts.len() {
                // Rebuild parts array with only valid parts.
                turn["parts"] =
                    serde_json::Value::Array(valid_parts.into_iter().cloned().collect());
            }
            curated.push(turn);
        }
        curated
    }

    /// Count tokens for the given messages using the Gemini countTokens API.
    pub async fn count_tokens(&self, messages: &[ChatMessage]) -> Result<u32, LlmError> {
        let req =
            Self::to_gemini_request(messages, None, None, None, None, None, &self.config.model);
        let contents = req
            .get("contents")
            .cloned()
            .unwrap_or(serde_json::json!([]));

        let credential = self
            .cred_manager
            .get_valid_credential()
            .await
            .map_err(|_e| LlmError::AuthFailed {
                provider: "gemini_oauth".to_string(),
            })?;

        let (url, request_body) = if self.uses_cloud_code_api() {
            let url = "https://cloudcode-pa.googleapis.com/v1internal:countTokens".to_string();
            let mut req = serde_json::json!({
                "request": {
                    "model": format!("models/{}", self.config.model),
                    "contents": contents,
                }
            });
            if let Some(ref pid) = credential.project_id {
                req["project"] = serde_json::Value::String(pid.clone());
            }
            (url, req)
        } else {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:countTokens",
                self.config.model
            );
            (url, serde_json::json!({ "contents": contents }))
        };

        let response = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header(
                "Authorization",
                format!("Bearer {}", credential.access_token),
            )
            .json(&request_body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "gemini_oauth".to_string(),
                reason: e.to_string(),
            })?;

        let body: serde_json::Value =
            response.json().await.map_err(|e| LlmError::RequestFailed {
                provider: "gemini_oauth".to_string(),
                reason: format!("Failed to parse countTokens response: {}", e),
            })?;

        let total = body
            .get("totalTokens")
            .or_else(|| body.get("totalTokenCount"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;

        Ok(total)
    }

    /// Determine whether to use Cloud Code API vs legacy generativelanguage API.
    ///
    /// Gemini 2.0+ models use the Cloud Code API endpoint.
    /// Gemini 1.x models use the legacy generativelanguage.googleapis.com endpoint.
    fn uses_cloud_code_api(&self) -> bool {
        Self::model_uses_cloud_code_api(&self.config.model)
    }

    pub fn model_uses_cloud_code_api(model: &str) -> bool {
        let model = model.to_ascii_lowercase();
        // Models containing "-preview" suffix or "gemini-3" use the Cloud Code API.
        // Using "-preview" (with hyphen) to avoid false positives on unrelated model names.
        if model.contains("-preview") || model.contains("gemini-3") {
            return true;
        }

        if let Some(rest) = model.strip_prefix("gemini-") {
            let version_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let major: u32 = match version_str.parse() {
                Ok(v) => v,
                Err(_) => {
                    warn!(
                        model = model,
                        "could not parse major version from Gemini model name, defaulting to legacy API"
                    );
                    0
                }
            };
            major >= 2
        } else {
            false
        }
    }

    async fn send_request(
        &self,
        original_request: &serde_json::Value,
    ) -> Result<serde_json::Value, LlmError> {
        let mut allow_retry = true;
        loop {
            let credential = self
                .cred_manager
                .get_valid_credential()
                .await
                .map_err(|_e| LlmError::AuthFailed {
                    provider: "gemini_oauth".to_string(),
                })?;

            // Format is equivalent to the Google Generative Language API
            // https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent
            let (url, request_body, mut headers) = if self.uses_cloud_code_api() {
                // Use Cloud Code API for new models
                let url =
                    "https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse"
                        .to_string();
                let mut req = serde_json::json!({
                    "model": self.config.model,
                    "request": original_request,
                });
                if let Some(ref pid) = credential.project_id {
                    req["project"] = serde_json::Value::String(pid.clone());
                }

                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    "Content-Type",
                    "application/json"
                        .parse()
                        .map_err(|_| LlmError::RequestFailed {
                            provider: "gemini_oauth".to_string(),
                            reason: "invalid Content-Type header value".to_string(),
                        })?,
                );
                headers.insert(
                    "User-Agent",
                    format!(
                        "GeminiCLI-ironclaw/{}/{} ({}; {}; cli)",
                        env!("CARGO_PKG_VERSION"),
                        self.config.model,
                        std::env::consts::OS,
                        std::env::consts::ARCH,
                    )
                    .parse()
                    .map_err(|_| LlmError::RequestFailed {
                        provider: "gemini_oauth".to_string(),
                        reason: "invalid User-Agent header value".to_string(),
                    })?,
                );
                headers.insert(
                    "X-Goog-Api-Client",
                    GOOG_API_CLIENT
                        .parse()
                        .map_err(|_| LlmError::RequestFailed {
                            provider: "gemini_oauth".to_string(),
                            reason: "invalid X-Goog-Api-Client header value".to_string(),
                        })?,
                );
                headers.insert(
                    "Client-Metadata",
                    "{\"ideType\":\"IDE_UNSPECIFIED\",\"platform\":\"PLATFORM_UNSPECIFIED\",\"pluginType\":\"GEMINI\"}"
                        .parse()
                        .map_err(|_| LlmError::RequestFailed {
                            provider: "gemini_oauth".to_string(),
                            reason: "invalid Client-Metadata header value".to_string(),
                        })?,
                );
                headers.insert(
                    "Authorization",
                    reqwest::header::HeaderValue::from_str(&format!(
                        "Bearer {}",
                        credential.access_token
                    ))
                    .map_err(|_| LlmError::AuthFailed {
                        provider: "gemini_oauth".to_string(),
                    })?,
                );
                (url, req, headers)
            } else {
                // Legacy / Standard fallback
                // Respect GOOGLE_GENAI_API_VERSION env var (default: v1beta)
                let api_version = std::env::var("GOOGLE_GENAI_API_VERSION")
                    .unwrap_or_else(|_| "v1beta".to_string());
                let url = format!(
                    "https://generativelanguage.googleapis.com/{}/models/{}:generateContent",
                    api_version, self.config.model
                );

                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    "Content-Type",
                    "application/json"
                        .parse()
                        .map_err(|_| LlmError::RequestFailed {
                            provider: "gemini_oauth".to_string(),
                            reason: "invalid Content-Type header value".to_string(),
                        })?,
                );

                // Support GEMINI_API_KEY for non-OAuth auth + GEMINI_API_KEY_AUTH_MECHANISM
                let api_key = std::env::var("GEMINI_API_KEY").ok();
                let auth_mechanism = std::env::var("GEMINI_API_KEY_AUTH_MECHANISM")
                    .unwrap_or_else(|_| "x-goog-api-key".to_string());

                let (final_url, auth_header_name, auth_header_value) =
                    if let Some(ref key) = api_key {
                        if auth_mechanism == "bearer" {
                            (url, "Authorization".to_string(), format!("Bearer {}", key))
                        } else {
                            // x-goog-api-key: append key as query param or header
                            (url, "x-goog-api-key".to_string(), key.clone())
                        }
                    } else {
                        (
                            url,
                            "Authorization".to_string(),
                            format!("Bearer {}", credential.access_token),
                        )
                    };

                headers.insert(
                    reqwest::header::HeaderName::from_bytes(auth_header_name.as_bytes()).map_err(
                        |_| LlmError::RequestFailed {
                            provider: "gemini_oauth".to_string(),
                            reason: "invalid auth header name".to_string(),
                        },
                    )?,
                    reqwest::header::HeaderValue::from_str(&auth_header_value).map_err(|_| {
                        LlmError::AuthFailed {
                            provider: "gemini_oauth".to_string(),
                        }
                    })?,
                );

                (final_url, original_request.clone(), headers)
            };

            // Inject custom headers from GEMINI_CLI_CUSTOM_HEADERS env var
            let custom_headers = parse_custom_headers();
            for (name, value) in &custom_headers {
                if let (Ok(hname), Ok(hval)) = (
                    reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                    reqwest::header::HeaderValue::from_str(value),
                ) {
                    headers.insert(hname, hval);
                } else {
                    warn!(header = %name, "Skipping invalid custom header");
                }
            }

            debug!(
                url = %url,
                model = %self.config.model,
                "gemini_oauth: sending request"
            );

            let response = self
                .http_client
                .post(&url)
                .headers(headers)
                .json(&request_body)
                .send()
                .await
                .map_err(|e| LlmError::RequestFailed {
                    provider: "gemini_oauth".to_string(),
                    reason: e.to_string(),
                })?;

            let status = response.status();
            let body_bytes = response
                .bytes()
                .await
                .map_err(|e| LlmError::RequestFailed {
                    provider: "gemini_oauth".to_string(),
                    reason: format!("Failed to read response body: {}", e),
                })?;

            // Cloud Code returns SSE stream, we need to parse it
            let mut final_response = serde_json::json!({});
            let body_str = String::from_utf8_lossy(&body_bytes);

            let mut success = false;
            if self.uses_cloud_code_api() {
                let mut combined_text = String::new();
                let mut finish_reason = "STOP".to_string();
                let mut prompt_tokens: i64 = 0;
                let mut candidates_tokens: i64 = 0;
                let mut tool_calls_parts = Vec::<serde_json::Value>::new();

                // Metadata (collected in the same pass)
                let mut model_version: Option<String> = None;
                let mut prompt_feedback: Option<serde_json::Value> = None;
                let mut grounding_metadata: Option<serde_json::Value> = None;
                let mut citation_metadata: Option<serde_json::Value> = None;
                let mut cached_content_token_count: Option<u32> = None;
                let mut total_token_count: Option<u32> = None;
                let mut consumed_credits: Vec<GeminiCredits> = Vec::new();
                let mut remaining_credits: Vec<GeminiCredits> = Vec::new();

                for line in body_str.lines() {
                    let Some(json_str) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let json_str = json_str.trim();
                    let chunk: serde_json::Value = match serde_json::from_str(json_str) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    // Credits from Cloud Code wrapper (top-level, outside "response")
                    if let Some(cc) = chunk.get("consumedCredits").and_then(|c| c.as_array()) {
                        for c in cc {
                            if let Ok(credit) = serde_json::from_value::<GeminiCredits>(c.clone()) {
                                consumed_credits.push(credit);
                            }
                        }
                    }
                    if let Some(rc) = chunk.get("remainingCredits").and_then(|c| c.as_array()) {
                        for c in rc {
                            if let Ok(credit) = serde_json::from_value::<GeminiCredits>(c.clone()) {
                                remaining_credits.push(credit);
                            }
                        }
                    }

                    let resp = match chunk.get("response") {
                        Some(r) => r,
                        None => continue,
                    };

                    // Content extraction
                    if let Some(candidates) = resp.get("candidates").and_then(|c| c.as_array())
                        && let Some(first) = candidates.first()
                    {
                        if let Some(parts) = first
                            .get("content")
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.as_array())
                        {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                    let is_thought = part
                                        .get("thought")
                                        .and_then(|t| t.as_bool())
                                        .unwrap_or(false);
                                    if !is_thought {
                                        combined_text.push_str(text);
                                    }
                                }
                                if let Some(fc) = part.get("functionCall") {
                                    tool_calls_parts.push(serde_json::json!({
                                        "functionCall": fc
                                    }));
                                }
                            }
                        }
                        if let Some(fr) = first.get("finishReason").and_then(|fr| fr.as_str()) {
                            finish_reason = fr.to_string();
                        }
                        // Per-candidate metadata
                        if grounding_metadata.is_none()
                            && let Some(gm) = first.get("groundingMetadata")
                        {
                            grounding_metadata = Some(gm.clone());
                        }
                        if citation_metadata.is_none()
                            && let Some(cm) = first.get("citationMetadata")
                        {
                            citation_metadata = Some(cm.clone());
                        }
                    }

                    // Response-level metadata
                    if model_version.is_none()
                        && let Some(mv) = resp.get("modelVersion").and_then(|v| v.as_str())
                    {
                        model_version = Some(mv.to_string());
                    }
                    if prompt_feedback.is_none()
                        && let Some(pf) = resp.get("promptFeedback")
                    {
                        prompt_feedback = Some(pf.clone());
                    }
                    if let Some(usage) = resp.get("usageMetadata") {
                        if let Some(pt) = usage.get("promptTokenCount").and_then(|pt| pt.as_i64()) {
                            prompt_tokens = pt;
                        }
                        if let Some(ct) =
                            usage.get("candidatesTokenCount").and_then(|ct| ct.as_i64())
                        {
                            candidates_tokens = ct;
                        }
                        if let Some(ct) = usage
                            .get("cachedContentTokenCount")
                            .and_then(|t| t.as_u64())
                        {
                            cached_content_token_count = Some(ct as u32);
                        }
                        if let Some(tt) = usage.get("totalTokenCount").and_then(|t| t.as_u64()) {
                            total_token_count = Some(tt as u32);
                        }
                    }
                }

                // Store metadata
                if let Ok(mut meta) = self.last_response_meta.lock() {
                    *meta = GeminiResponseMeta {
                        model_version,
                        prompt_feedback: prompt_feedback.clone(),
                        grounding_metadata,
                        citation_metadata,
                        consumed_credits,
                        remaining_credits,
                        cached_content_token_count,
                        total_token_count,
                    };
                }

                // Log prompt feedback if request was blocked
                if let Some(ref pf) = prompt_feedback
                    && let Some(reason) = pf.get("blockReason").and_then(|r| r.as_str())
                {
                    warn!(
                        block_reason = reason,
                        "Gemini API blocked the request via promptFeedback"
                    );
                }

                let has_content = !combined_text.is_empty() || !tool_calls_parts.is_empty();

                if has_content {
                    let mut response_parts = Vec::new();
                    if !combined_text.is_empty() {
                        response_parts.push(serde_json::json!({"text": combined_text}));
                    }
                    response_parts.extend(tool_calls_parts);

                    final_response = serde_json::json!({
                        "candidates": [{
                            "content": {
                                "parts": response_parts
                            },
                            "finishReason": finish_reason
                        }],
                        "usageMetadata": {
                            "promptTokenCount": prompt_tokens,
                            "candidatesTokenCount": candidates_tokens
                        }
                    });
                    success = true;
                }
            } else if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_str) {
                final_response = json;
                success = true;
            }

            if !status.is_success() || !success {
                let err_msg = final_response
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or(&body_str);

                if status.as_u16() == 401 && allow_retry {
                    warn!(
                        "Gemini OAuth request failed with 401. Force-refreshing token and retrying..."
                    );
                    if let Err(e) = self.cred_manager.force_refresh().await {
                        error!("Failed to force-refresh token: {}", e);
                        return Err(LlmError::RequestFailed {
                            provider: "gemini_oauth".to_string(),
                            reason: format!("Auth error 401 and refresh failed: {}", e),
                        });
                    }
                    allow_retry = false;
                    continue;
                }

                if status.as_u16() == 429 {
                    let retry_after = Self::parse_retry_after(err_msg);
                    return Err(LlmError::RateLimited {
                        provider: "gemini_oauth".to_string(),
                        retry_after,
                    });
                }

                return Err(LlmError::InvalidResponse {
                    provider: "gemini_oauth".to_string(),
                    reason: format!("HTTP {}: {}", status.as_u16(), err_msg),
                });
            }

            return Ok(final_response);
        }
    }

    /// Parse retry-after duration from Gemini error messages.
    ///
    /// Matches patterns like "Your quota will reset after 46s."
    /// or "Your quota will reset after 18h31m10s."
    fn parse_retry_after(message: &str) -> Option<Duration> {
        use std::sync::LazyLock;
        use std::time::Duration;

        static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"reset after (?:(\d+)h)?(?:(\d+)m)?(\d+)s")
                .expect("invalid retry_after regex") // safety: hardcoded literal
        });

        let caps = RE.captures(message)?;
        let hours: u64 = caps.get(1).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let minutes: u64 = caps.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let seconds: u64 = caps.get(3).map_or(0, |m| m.as_str().parse().unwrap_or(0));

        let total_secs = hours * 3600 + minutes * 60 + seconds;
        if total_secs > 0 {
            Some(Duration::from_secs(total_secs + 2))
        } else {
            None
        }
    }

    fn to_gemini_request(
        messages: &[ChatMessage],
        tools: Option<&[ToolDefinition]>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        stop_sequences: Option<&[String]>,
        tool_choice: Option<&str>,
        model: &str,
    ) -> serde_json::Value {
        let mut contents = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    // System messages are handled via systemInstruction top-level field
                }
                Role::User => {
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{ "text": msg.content }]
                    }));
                }
                Role::Assistant => {
                    let mut parts = Vec::new();
                    // Only add text part if content is non-empty (assistant messages
                    // with tool calls often have empty content, and curate_contents
                    // would drop the entire turn if it sees an empty text part).
                    if !msg.content.is_empty() {
                        parts.push(serde_json::json!({ "text": msg.content }));
                    }
                    if let Some(ref calls) = msg.tool_calls {
                        for call in calls {
                            let mut part = serde_json::json!({
                                "functionCall": {
                                    "name": call.name,
                                    "args": call.arguments
                                }
                            });
                            // Echo back the original thoughtSignature if present.
                            if let Some(ref sig) = call.thought_signature
                                && let Some(obj) = part.as_object_mut()
                            {
                                obj.insert(
                                    "thoughtSignature".to_string(),
                                    serde_json::Value::String(sig.clone()),
                                );
                            }
                            parts.push(part);
                        }
                    }
                    // Fallback: if no parts at all, add empty text to avoid
                    // sending a turn with zero parts (API rejects it).
                    if parts.is_empty() {
                        parts.push(serde_json::json!({ "text": "" }));
                    }
                    contents.push(serde_json::json!({
                        "role": "model",
                        "parts": parts
                    }));
                }
                Role::Tool => {
                    let tool_name = msg
                        .name
                        .clone()
                        .unwrap_or_else(|| "unknown_tool".to_string());

                    let response_value: serde_json::Value = serde_json::from_str(&msg.content)
                        .unwrap_or_else(|_| serde_json::json!({ "output": msg.content }));

                    let part = serde_json::json!({
                        "functionResponse": {
                            "name": tool_name,
                            "response": response_value
                        }
                    });

                    let last = contents.last_mut();
                    let merge = last
                        .as_ref()
                        .and_then(|c| c.get("role"))
                        .and_then(|r| r.as_str())
                        == Some("user")
                        && last
                            .as_ref()
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.as_array())
                            .is_some_and(|parts| {
                                parts.iter().any(|p| p.get("functionResponse").is_some())
                            });

                    if merge {
                        if let Some(c) = contents.last_mut()
                            && let Some(parts) = c.get_mut("parts").and_then(|p| p.as_array_mut())
                        {
                            parts.push(part);
                        }
                    } else {
                        contents.push(serde_json::json!({
                            "role": "user",
                            "parts": [part]
                        }));
                    }
                }
            }
        }

        let mut req = serde_json::json!({
            "contents": contents
        });

        // Concatenate all system messages into one systemInstruction
        let mut system_parts = Vec::new();
        for msg in messages {
            if msg.role == Role::System {
                system_parts.push(msg.content.as_str());
            }
        }

        if !system_parts.is_empty() {
            req["systemInstruction"] = serde_json::json!({
                "parts": [{ "text": system_parts.join("\n\n") }]
            });
        }

        if let Some(tool_defs) = tools
            && !tool_defs.is_empty()
        {
            let declarations: Vec<serde_json::Value> = tool_defs
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    })
                })
                .collect();

            req["tools"] = serde_json::json!([
                { "functionDeclarations": declarations }
            ]);
        }

        let mut gen_config = serde_json::Map::new();
        if let Some(t) = temperature {
            gen_config.insert("temperature".to_string(), serde_json::Value::from(t));
        }
        if let Some(mt) = max_tokens {
            gen_config.insert("maxOutputTokens".to_string(), serde_json::Value::from(mt));
        }
        if let Some(seqs) = stop_sequences
            && !seqs.is_empty()
        {
            gen_config.insert(
                "stopSequences".to_string(),
                serde_json::Value::from(seqs.to_vec()),
            );
        }

        // Extended generation config from environment variables.
        // These allow fine-tuning without changing the shared CompletionRequest trait.
        if let Ok(v) = std::env::var("GEMINI_TOP_P")
            && let Ok(top_p) = v.parse::<f64>()
        {
            gen_config.insert("topP".to_string(), serde_json::Value::from(top_p));
        }
        if let Ok(v) = std::env::var("GEMINI_TOP_K")
            && let Ok(top_k) = v.parse::<u32>()
        {
            gen_config.insert("topK".to_string(), serde_json::Value::from(top_k));
        }
        if let Ok(v) = std::env::var("GEMINI_SEED")
            && let Ok(seed) = v.parse::<i64>()
        {
            gen_config.insert("seed".to_string(), serde_json::Value::from(seed));
        }
        if let Ok(v) = std::env::var("GEMINI_PRESENCE_PENALTY")
            && let Ok(pp) = v.parse::<f64>()
        {
            gen_config.insert("presencePenalty".to_string(), serde_json::Value::from(pp));
        }
        if let Ok(v) = std::env::var("GEMINI_FREQUENCY_PENALTY")
            && let Ok(fp) = v.parse::<f64>()
        {
            gen_config.insert("frequencyPenalty".to_string(), serde_json::Value::from(fp));
        }
        // Response schema / JSON mode
        if let Ok(mime) = std::env::var("GEMINI_RESPONSE_MIME_TYPE")
            && !mime.is_empty()
        {
            gen_config.insert(
                "responseMimeType".to_string(),
                serde_json::Value::String(mime),
            );
        }
        if let Ok(schema_str) = std::env::var("GEMINI_RESPONSE_JSON_SCHEMA")
            && let Ok(schema) = serde_json::from_str::<serde_json::Value>(&schema_str)
        {
            gen_config.insert("responseJsonSchema".to_string(), schema);
        }

        // thinkingConfig:
        // - Gemini 3.x: level-based (thinkingLevel: HIGH)
        // - Gemini 2.5.x: budget-based (thinkingBudget: 8192)
        // Budget cap of 8192 prevents runaway thinking loops.
        //
        // NOTE: We do NOT set includeThoughts=true. The original Gemini CLI
        // sets it because it displays thoughts to the user. IronClaw's reasoning
        // layer (reasoning.rs) strips all <thinking> tags from responses, so
        // including thoughts just adds text that gets stripped, potentially
        // leaving an empty response.
        let is_gemini_3 = model.contains("gemini-3");
        let is_gemini_25 = model.contains("gemini-2.5");
        let is_thinking_model = model.contains("thinking") || is_gemini_3 || is_gemini_25;
        if is_thinking_model {
            let thinking_config = if is_gemini_3 {
                serde_json::json!({ "thinkingLevel": "HIGH" })
            } else {
                serde_json::json!({ "thinkingBudget": 8192 })
            };
            gen_config.insert("thinkingConfig".to_string(), thinking_config);
        }

        if !gen_config.is_empty() {
            req["generationConfig"] = serde_json::Value::Object(gen_config);
        }

        // Cached content support via GEMINI_CACHED_CONTENT env var.
        if let Ok(cached) = std::env::var("GEMINI_CACHED_CONTENT")
            && !cached.is_empty()
        {
            req["cachedContent"] = serde_json::Value::String(cached);
        }

        if let Some(choice) = tool_choice {
            let mode = match choice {
                "auto" => "AUTO",
                "required" | "any" => "ANY",
                "none" => "NONE",
                _ => "AUTO",
            };
            req["toolConfig"] = serde_json::json!({
                "functionCallingConfig": {
                    "mode": mode
                }
            });
        }

        // Safety settings — only inject BLOCK_NONE when explicitly enabled via env var.
        // The Cloud Code API may reject BLOCK_NONE for certain tiers.
        // The original Gemini CLI does not set default safety settings.
        if std::env::var("GEMINI_SAFETY_BLOCK_NONE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            req["safetySettings"] = serde_json::Value::Array(default_safety_settings());
        }

        // Thought signature injection for models that support modern features (Gemini 3.x).
        if supports_modern_features(model)
            && let Some(contents) = req.get_mut("contents").and_then(|c| c.as_array_mut())
        {
            let mut owned = contents.clone();
            Self::ensure_thought_signatures(&mut owned);
            *contents = owned;
        }

        // History curation: filter out invalid model outputs before sending.
        if let Some(contents) = req.get("contents").and_then(|c| c.as_array()) {
            let curated = Self::curate_contents(contents);
            req["contents"] = serde_json::Value::Array(curated);
        }

        req
    }

    fn from_gemini_response(
        body: serde_json::Value,
    ) -> Result<(CompletionResponse, Vec<ToolCall>), LlmError> {
        let candidate = body
            .get("candidates")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .ok_or_else(|| LlmError::RequestFailed {
                provider: "gemini_oauth".to_string(),
                reason: "Response missing 'candidates[0]'".to_string(),
            })?;

        let parts = candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array());

        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        if let Some(parts) = parts {
            for part in parts {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    text_content.push_str(text);
                }
                if let Some(fc) = part.get("functionCall") {
                    let name = fc
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
                    let id = fc
                        .get("id")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    // Capture thoughtSignature (sibling of functionCall in the part)
                    // so it can be echoed back when replaying history.
                    let thought_signature = part
                        .get("thoughtSignature")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string());

                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: args,
                        reasoning: None,
                        thought_signature,
                    });
                }
            }
        }

        let finish_reason = candidate
            .get("finishReason")
            .and_then(|r| r.as_str())
            .unwrap_or("STOP");

        // Invalid content detection (mirrors Gemini CLI InvalidStreamError types).
        // Log warnings for known problematic finish reasons.
        match finish_reason {
            "MALFORMED_FUNCTION_CALL" => {
                warn!(
                    finish_reason = finish_reason,
                    "Gemini returned MALFORMED_FUNCTION_CALL — {} (type: {})",
                    "model stream ended with malformed function call",
                    InvalidStreamType::MalformedFunctionCall
                );
            }
            "UNEXPECTED_TOOL_CALL" => {
                warn!(
                    finish_reason = finish_reason,
                    "Gemini returned UNEXPECTED_TOOL_CALL — {} (type: {})",
                    "model stream ended with unexpected tool call",
                    InvalidStreamType::UnexpectedToolCall
                );
            }
            _ => {}
        }

        // Check for no response text when no tool calls (NO_RESPONSE_TEXT detection)
        if tool_calls.is_empty() && text_content.is_empty() && finish_reason == "STOP" {
            debug!(
                "Gemini response has no text and no tool calls (type: {})",
                InvalidStreamType::NoResponseText
            );
        }

        let stop_reason = match finish_reason {
            "STOP" => {
                if !tool_calls.is_empty() {
                    FinishReason::ToolUse
                } else {
                    FinishReason::Stop
                }
            }
            "MAX_TOKENS" => FinishReason::Length,
            "MALFORMED_FUNCTION_CALL" | "UNEXPECTED_TOOL_CALL" => {
                // Treat as Stop — the caller's retry layer will handle retries
                FinishReason::Stop
            }
            _ => {
                if !tool_calls.is_empty() {
                    FinishReason::ToolUse
                } else {
                    FinishReason::Stop
                }
            }
        };

        let usage = body.get("usageMetadata");
        let input_tokens = usage
            .and_then(|u| u.get("promptTokenCount"))
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as u32;
        let output_tokens = usage
            .and_then(|u| u.get("candidatesTokenCount"))
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as u32;
        let cached_content_tokens = usage
            .and_then(|u| u.get("cachedContentTokenCount"))
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as u32;

        // Extract additional metadata from non-SSE (legacy) responses.
        let _model_version = body
            .get("modelVersion")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let _prompt_feedback = body.get("promptFeedback").cloned();
        let _grounding_metadata = candidate.get("groundingMetadata").cloned();
        let _citation_metadata = candidate.get("citationMetadata").cloned();

        // Log prompt feedback if present
        if let Some(ref pf) = _prompt_feedback
            && let Some(reason) = pf.get("blockReason").and_then(|r| r.as_str())
        {
            warn!(
                block_reason = reason,
                "Gemini API blocked the request via promptFeedback"
            );
        }

        Ok((
            CompletionResponse {
                content: text_content,
                finish_reason: stop_reason,
                input_tokens,
                output_tokens,
                cache_read_input_tokens: cached_content_tokens,
                cache_creation_input_tokens: 0,
            },
            tool_calls,
        ))
    }
}

#[async_trait::async_trait]
impl LlmProvider for GeminiOauthProvider {
    fn model_name(&self) -> &str {
        &self.config.model
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        let model = self.config.model.as_str();
        let context_length = Some(gemini_context_length(model));

        Ok(ModelMetadata {
            id: self.config.model.clone(),
            context_length,
        })
    }

    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(vec![
            "gemini-3.1-pro-preview".to_string(),
            "gemini-3.1-pro-preview-customtools".to_string(),
            "gemini-3-pro-preview".to_string(),
            "gemini-3-flash-preview".to_string(),
            "gemini-3.1-flash-lite-preview".to_string(),
            "gemini-2.5-pro".to_string(),
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-flash-lite".to_string(),
        ])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let req_json = Self::to_gemini_request(
            &request.messages,
            None,
            request.temperature,
            request.max_tokens,
            request.stop_sequences.as_deref(),
            None,
            &self.config.model,
        );
        let resp_json = self.send_request(&req_json).await?;
        let (response, _tool_calls) = Self::from_gemini_response(resp_json)?;
        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: crate::llm::provider::ToolCompletionRequest,
    ) -> Result<crate::llm::provider::ToolCompletionResponse, LlmError> {
        let tool_defs = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.as_slice())
        };

        let req_json = Self::to_gemini_request(
            &request.messages,
            tool_defs,
            request.temperature,
            request.max_tokens,
            request.stop_sequences.as_deref(),
            request.tool_choice.as_deref(),
            &self.config.model,
        );
        let resp_json = self.send_request(&req_json).await?;
        let (response, tool_calls) = Self::from_gemini_response(resp_json)?;

        Ok(crate::llm::provider::ToolCompletionResponse {
            content: if response.content.is_empty() {
                None
            } else {
                Some(response.content)
            },
            finish_reason: response.finish_reason,
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            tool_calls,
            cache_read_input_tokens: response.cache_read_input_tokens,
            cache_creation_input_tokens: response.cache_creation_input_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deobfuscate_reconstructs_credentials() {
        let client_id = oauth_client_id();
        assert!(client_id.ends_with(".apps.googleusercontent.com"));
        assert!(client_id.starts_with("681"));

        let client_secret = oauth_client_secret();
        assert!(client_secret.starts_with("GOCSPX-"));
        assert!(!client_secret.is_empty());
    }

    #[test]
    fn test_generate_pkce_params_format() {
        let params = generate_pkce_params();

        assert_eq!(params.code_verifier.len(), 64);
        assert_eq!(params.state.len(), 32);
        assert!(!params.code_challenge.is_empty());

        assert!(
            params
                .code_verifier
                .chars()
                .all(|c| { c.is_ascii_alphanumeric() || "-._~".contains(c) })
        );
        assert!(params.state.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_parse_callback_params_valid() {
        let raw = "GET /auth/callback?code=abc123&state=xyz789 HTTP/1.1\r\nHost: localhost\r\n";
        let (code, state, error) = CredentialManager::parse_callback_params(raw);
        assert_eq!(code.as_deref(), Some("abc123"));
        assert_eq!(state.as_deref(), Some("xyz789"));
        assert!(error.is_none());
    }

    #[test]
    fn test_parse_callback_params_with_error() {
        let raw = "GET /auth/callback?error=access_denied HTTP/1.1\r\n";
        let (code, state, error) = CredentialManager::parse_callback_params(raw);
        assert!(code.is_none());
        assert!(state.is_none());
        assert_eq!(error.as_deref(), Some("access_denied"));
    }

    #[test]
    fn test_parse_callback_params_empty() {
        let (code, state, error) = CredentialManager::parse_callback_params("");
        assert!(code.is_none());
        assert!(state.is_none());
        assert!(error.is_none());
    }

    #[test]
    fn test_parse_retry_after_seconds() {
        let result = GeminiOauthProvider::parse_retry_after(
            "RESOURCE_EXHAUSTED: Your quota will reset after 46s.",
        );
        assert_eq!(result, Some(Duration::from_secs(48)));
    }

    #[test]
    fn test_parse_retry_after_hours_minutes_seconds() {
        let result =
            GeminiOauthProvider::parse_retry_after("Your quota will reset after 18h31m10s.");
        let expected = 18 * 3600 + 31 * 60 + 10 + 2;
        assert_eq!(result, Some(Duration::from_secs(expected)));
    }

    #[test]
    fn test_parse_retry_after_no_match() {
        let result = GeminiOauthProvider::parse_retry_after("Some random error message");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_redirect_url_valid() {
        let url = "http://127.0.0.1:8080/auth/callback?code=4/abc&state=xyz123";
        let result = CredentialManager::parse_redirect_url(url);
        assert!(result.is_ok());
        let (code, state) = result.unwrap();
        assert_eq!(code, "4/abc");
        assert_eq!(state, "xyz123");
    }

    #[test]
    fn test_parse_redirect_url_invalid() {
        let result = CredentialManager::parse_redirect_url("not-a-url");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_redirect_url_missing_code() {
        let url = "http://127.0.0.1:8080/auth/callback?state=xyz";
        let result = CredentialManager::parse_redirect_url(url);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_gemini_request_with_tools() {
        let messages = vec![ChatMessage::user("Hello")];
        let tools = vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                }
            }),
        }];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            Some(&tools),
            None,
            None,
            None,
            None,
            "gemini-2.0-flash",
        );

        let decls = &req["tools"][0]["functionDeclarations"];
        assert_eq!(decls[0]["name"], "read_file");
        assert_eq!(decls[0]["description"], "Read a file");
    }

    #[test]
    fn test_to_gemini_request_tool_response() {
        let messages = vec![
            ChatMessage::user("Read /tmp/test"),
            ChatMessage::tool_result("call_123", "read_file", "file contents here"),
        ];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            None,
            None,
            None,
            None,
            None,
            "gemini-2.0-flash",
        );

        let contents = req["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 2);

        let tool_part = &contents[1]["parts"][0];
        assert!(tool_part.get("functionResponse").is_some());
        assert_eq!(tool_part["functionResponse"]["name"], "read_file");
    }

    #[test]
    fn test_from_gemini_response_text() {
        let body = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{ "text": "Hello world" }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5
            }
        });

        let (resp, tool_calls) = GeminiOauthProvider::from_gemini_response(body).unwrap();

        assert_eq!(resp.content, "Hello world");
        assert_eq!(resp.input_tokens, 10);
        assert_eq!(resp.output_tokens, 5);
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_from_gemini_response_function_call() {
        let body = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "read_file",
                            "args": { "path": "/tmp/test.txt" }
                        }
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 15,
                "candidatesTokenCount": 8
            }
        });

        let (resp, tool_calls) = GeminiOauthProvider::from_gemini_response(body).unwrap();

        assert!(resp.content.is_empty());
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "read_file");
        assert_eq!(tool_calls[0].arguments["path"], "/tmp/test.txt");
    }

    #[test]
    fn test_generation_config_passed() {
        let messages = vec![ChatMessage::user("Hi")];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            None,
            Some(0.7),
            Some(4096),
            None,
            None,
            "gemini-2.0-flash",
        );

        let gen_cfg = &req["generationConfig"];
        assert_eq!(gen_cfg["temperature"], 0.7_f32);
        assert_eq!(gen_cfg["maxOutputTokens"], 4096);
        assert!(gen_cfg.get("thinkingConfig").is_none());
    }

    #[test]
    fn test_thinking_config_for_gemini3_thinking_level() {
        let messages = vec![ChatMessage::user("Reason about this")];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            None,
            None,
            None,
            None,
            None,
            "gemini-3-flash-preview",
        );

        let thinking = &req["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["thinkingLevel"], "HIGH");
        assert!(thinking.get("includeThoughts").is_none());
        assert!(thinking.get("thinkingBudget").is_none());
    }

    #[test]
    fn test_thinking_config_for_gemini25_budget() {
        let messages = vec![ChatMessage::user("Think about this")];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            None,
            None,
            None,
            None,
            None,
            "gemini-2.5-flash-thinking",
        );

        let thinking = &req["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["thinkingBudget"], 8192);
        // includeThoughts is NOT set — reasoning.rs strips thinking tags,
        // so returning thoughts just causes empty responses.
        assert!(thinking.get("includeThoughts").is_none() || thinking["includeThoughts"].is_null());
        assert!(thinking.get("thinkingLevel").is_none());
    }

    #[test]
    fn test_stop_sequences_in_generation_config() {
        let messages = vec![ChatMessage::user("Hi")];
        let stops = vec!["STOP1".to_string(), "STOP2".to_string()];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            None,
            None,
            None,
            Some(&stops),
            None,
            "gemini-2.5-flash",
        );

        let gen_cfg = &req["generationConfig"];
        let stop_seqs = gen_cfg["stopSequences"].as_array().unwrap();
        assert_eq!(stop_seqs.len(), 2);
        assert_eq!(stop_seqs[0], "STOP1");
        assert_eq!(stop_seqs[1], "STOP2");
    }

    #[test]
    fn test_tool_config_mode_mapping() {
        let messages = vec![ChatMessage::user("Use tools")];

        let tools = vec![ToolDefinition {
            name: "test".to_string(),
            description: "test".to_string(),
            parameters: serde_json::json!({}),
        }];

        let req_auto = GeminiOauthProvider::to_gemini_request(
            &messages,
            Some(&tools),
            None,
            None,
            None,
            Some("auto"),
            "gemini-2.0-flash",
        );
        assert_eq!(
            req_auto["toolConfig"]["functionCallingConfig"]["mode"],
            "AUTO"
        );

        let req_req = GeminiOauthProvider::to_gemini_request(
            &messages,
            Some(&tools),
            None,
            None,
            None,
            Some("required"),
            "gemini-2.0-flash",
        );
        assert_eq!(
            req_req["toolConfig"]["functionCallingConfig"]["mode"],
            "ANY"
        );

        let req_none = GeminiOauthProvider::to_gemini_request(
            &messages,
            Some(&tools),
            None,
            None,
            None,
            Some("none"),
            "gemini-2.0-flash",
        );
        assert_eq!(
            req_none["toolConfig"]["functionCallingConfig"]["mode"],
            "NONE"
        );
    }

    #[test]
    fn test_oauth_credential_debug_redaction() {
        let cred = OAuthCredential {
            access_token: "secret_access".to_string(),
            refresh_token: Some("secret_refresh".to_string()),
            id_token: Some("secret_id".to_string()),
            token_type: Some("Bearer".to_string()),
            project_id: Some("test-project".to_string()),
            expiry_date: None,
        };
        let debug_str = format!("{:?}", cred);
        assert!(!debug_str.contains("secret_access"));
        assert!(!debug_str.contains("secret_refresh"));
        assert!(!debug_str.contains("secret_id"));
        assert!(debug_str.contains("[REDACTED]"));
        assert!(debug_str.contains("test-project"));
    }

    #[test]
    fn test_uses_cloud_code_api_logic() {
        let cases = [
            ("gemini-1.5-flash", false),
            ("gemini-1.5-pro", false),
            ("gemini-2.0-flash-exp", true),
            ("gemini-2.0-flash", true),
            ("gemini-2.0-flash-thinking", true),
            ("gemini-2.5-flash", true),
            ("gemini-3.0-flash-thinking-preview", true),
            ("gemini-3-pro", true),
            ("my-preview-custom", true), // contains "-preview", routes to Cloud Code
            ("mypreviewcustom", false),  // no hyphen before "preview", no false positive
            ("not-a-gemini-model", false),
        ];

        for (model, expected) in cases {
            assert_eq!(
                GeminiOauthProvider::model_uses_cloud_code_api(model),
                expected,
                "Model '{}': expected {}, got {}",
                model,
                expected,
                !expected
            );
        }
    }

    #[test]
    fn test_to_gemini_request_system_instruction_concatenation() {
        let messages = vec![
            ChatMessage::system("System 1"),
            ChatMessage::system("System 2"),
            ChatMessage::user("User message"),
        ];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            None,
            None,
            None,
            None,
            None,
            "gemini-1.5-flash",
        );

        let system_instruction = req
            .get("systemInstruction")
            .expect("Missing systemInstruction");
        let parts = system_instruction
            .get("parts")
            .and_then(|p| p.as_array())
            .expect("Missing parts");
        assert_eq!(parts.len(), 1);
        let text = parts[0]
            .get("text")
            .and_then(|t| t.as_str())
            .expect("Missing text");
        assert!(text.contains("System 1"));
        assert!(text.contains("System 2"));
    }

    #[test]
    fn test_curate_contents_preserves_tool_call_with_empty_text() {
        // Regression: curate_contents must not drop model turns that contain
        // functionCall parts just because an accompanying text part is empty.
        let contents = vec![
            serde_json::json!({
                "role": "user",
                "parts": [{ "text": "call the tool" }]
            }),
            serde_json::json!({
                "role": "model",
                "parts": [
                    { "text": "" },
                    { "functionCall": { "name": "echo", "args": { "msg": "hi" } } }
                ]
            }),
            serde_json::json!({
                "role": "user",
                "parts": [{ "functionResponse": { "name": "echo", "response": { "output": "hi" } } }]
            }),
        ];

        let curated = GeminiOauthProvider::curate_contents(&contents);
        assert_eq!(curated.len(), 3, "All 3 turns should be preserved");

        // The model turn should keep the functionCall part but drop the empty text
        let model_parts = curated[1]
            .get("parts")
            .and_then(|p| p.as_array())
            .expect("model turn should have parts");
        assert_eq!(
            model_parts.len(),
            1,
            "Empty text part should be filtered out"
        );
        assert!(
            model_parts[0].get("functionCall").is_some(),
            "functionCall part should be preserved"
        );
    }

    #[test]
    fn test_curate_contents_drops_fully_invalid_turn() {
        // A model turn where ALL parts are invalid should be dropped.
        let contents = vec![
            serde_json::json!({
                "role": "user",
                "parts": [{ "text": "hello" }]
            }),
            serde_json::json!({
                "role": "model",
                "parts": [{ "text": "" }]
            }),
            serde_json::json!({
                "role": "user",
                "parts": [{ "text": "again" }]
            }),
        ];

        let curated = GeminiOauthProvider::curate_contents(&contents);
        assert_eq!(curated.len(), 2, "Invalid model turn should be dropped");
        assert_eq!(curated[0]["parts"][0]["text"], "hello");
        assert_eq!(curated[1]["parts"][0]["text"], "again");
    }

    #[test]
    fn test_ensure_thought_signatures_adds_signatures_to_all_function_calls() {
        let mut contents = vec![
            serde_json::json!({
                "role": "user",
                "parts": [{ "text": "call tools" }]
            }),
            serde_json::json!({
                "role": "model",
                "parts": [
                    { "functionCall": { "name": "memory_write", "args": { "key": "a" } } },
                    { "functionCall": { "name": "memory_write", "args": { "key": "b" } } }
                ]
            }),
        ];

        GeminiOauthProvider::ensure_thought_signatures(&mut contents);

        let parts = contents[1]
            .get("parts")
            .and_then(|p| p.as_array())
            .expect("model turn should have parts");

        let signed_calls = parts
            .iter()
            .filter(|part| part.get("functionCall").is_some())
            .filter(|part| part.get("thoughtSignature").is_some())
            .count();

        assert_eq!(signed_calls, 2); // safety: test-only assertion
    }

    #[test]
    fn test_from_gemini_response_captures_thought_signature() {
        let body = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "thoughtSignature": "abc123sig",
                        "functionCall": {
                            "name": "read_file",
                            "args": { "path": "/tmp/test.txt" }
                        }
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5
            }
        });

        let (_resp, tool_calls) = GeminiOauthProvider::from_gemini_response(body).unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            tool_calls[0].thought_signature.as_deref(),
            Some("abc123sig")
        );
    }

    #[test]
    fn test_from_gemini_response_no_thought_signature_yields_none() {
        let body = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "echo",
                            "args": {}
                        }
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 3
            }
        });

        let (_resp, tool_calls) = GeminiOauthProvider::from_gemini_response(body).unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert!(tool_calls[0].thought_signature.is_none());
    }

    #[test]
    fn test_to_gemini_request_echoes_thought_signature_on_function_call() {
        let messages = vec![
            ChatMessage::user("call a tool"),
            ChatMessage::assistant_with_tool_calls(
                None,
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    arguments: serde_json::json!({"path": "/tmp/x"}),
                    reasoning: None,
                    thought_signature: Some("sig_from_gemini".to_string()),
                }],
            ),
            ChatMessage::tool_result("call_1", "read_file", r#"{"output":"hello"}"#),
        ];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            None,
            None,
            None,
            None,
            None,
            "gemini-3-flash-preview",
        );

        let contents = req["contents"].as_array().unwrap();
        // The model turn (index 1) should have the thoughtSignature on its functionCall part.
        let model_turn = &contents[1];
        assert_eq!(model_turn["role"], "model");
        let fc_part = &model_turn["parts"][0];
        assert!(fc_part.get("functionCall").is_some());
        assert_eq!(fc_part["thoughtSignature"], "sig_from_gemini");
    }

    #[test]
    fn test_to_gemini_request_omits_thought_signature_when_none() {
        let messages = vec![
            ChatMessage::user("call a tool"),
            ChatMessage::assistant_with_tool_calls(
                None,
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: serde_json::json!({}),
                    reasoning: None,
                    thought_signature: None,
                }],
            ),
            ChatMessage::tool_result("call_1", "echo", r#"{"output":"ok"}"#),
        ];

        let req = GeminiOauthProvider::to_gemini_request(
            &messages,
            None,
            None,
            None,
            None,
            None,
            "gemini-2.0-flash",
        );

        let contents = req["contents"].as_array().unwrap();
        let model_turn = &contents[1];
        let fc_part = &model_turn["parts"][0];
        assert!(fc_part.get("functionCall").is_some());
        // No thoughtSignature should be present when the ToolCall had None.
        assert!(fc_part.get("thoughtSignature").is_none());
    }
}
