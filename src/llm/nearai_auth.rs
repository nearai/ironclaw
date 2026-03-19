use secrecy::{ExposeSecret, SecretString};

use crate::llm::LlmError;
use crate::llm::session::SessionManager;

/// Resolve the active NEAR AI bearer token only if already available.
///
/// Unlike [`resolve_nearai_bearer_token`], this helper is side-effect free:
/// it never triggers an interactive login flow.
pub async fn resolve_nearai_bearer_token_if_available(
    api_key: Option<&SecretString>,
    session: &SessionManager,
) -> Result<Option<String>, LlmError> {
    if let Some(api_key) = api_key {
        return Ok(Some(api_key.expose_secret().to_string()));
    }

    if session.has_token().await {
        let token = session.get_token().await?;
        return Ok(Some(token.expose_secret().to_string()));
    }

    if let Ok(key) = std::env::var("NEARAI_API_KEY")
        && !key.is_empty()
    {
        return Ok(Some(key));
    }

    Ok(None)
}

/// Resolve the active NEAR AI bearer token.
///
/// Priority order:
/// 1. Explicit API key from resolved config
/// 2. Existing session token
/// 3. Interactive session authentication
/// 4. `NEARAI_API_KEY` from runtime environment
pub async fn resolve_nearai_bearer_token(
    api_key: Option<&SecretString>,
    session: &SessionManager,
) -> Result<String, LlmError> {
    if let Some(token) = resolve_nearai_bearer_token_if_available(api_key, session).await? {
        return Ok(token);
    }

    session.ensure_authenticated().await?;

    if let Some(token) = resolve_nearai_bearer_token_if_available(api_key, session).await? {
        return Ok(token);
    }

    Err(LlmError::AuthFailed {
        provider: "nearai".to_string(),
    })
}
