//! Shared test helpers for OpenAI Codex provider tests.

use crate::config::OpenAiCodexConfig;

/// Build a minimal JWT for testing (header.payload.signature).
pub(crate) fn make_test_jwt(account_id: &str) -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let header = engine.encode(b"{\"alg\":\"RS256\",\"typ\":\"JWT\"}");
    let payload_json = serde_json::json!({
        "sub": "user123",
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account_id,
        },
    });
    let payload = engine.encode(payload_json.to_string().as_bytes());
    let sig = engine.encode(b"fake-signature");
    format!("{header}.{payload}.{sig}")
}

/// Build a test `OpenAiCodexConfig` with a given session path.
pub(crate) fn test_codex_config(session_path: std::path::PathBuf) -> OpenAiCodexConfig {
    OpenAiCodexConfig {
        model: "gpt-5.3-codex".to_string(),
        auth_endpoint: "https://auth.openai.com".to_string(),
        api_base_url: "https://chatgpt.com/backend-api/codex".to_string(),
        client_id: "test_client_id".to_string(),
        session_path,
        token_refresh_margin_secs: 300,
    }
}
