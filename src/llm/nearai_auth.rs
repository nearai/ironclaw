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

    if let Some(key) = crate::config::helpers::env_or_override("NEARAI_API_KEY") {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::{ENV_MUTEX, set_runtime_env};
    use crate::llm::session::SessionConfig;

    struct EnvGuard(&'static str, Option<String>);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: tests hold ENV_MUTEX while mutating the process environment.
            unsafe {
                match &self.1 {
                    Some(value) => std::env::set_var(self.0, value),
                    None => std::env::remove_var(self.0),
                }
            }
            set_runtime_env(self.0, "");
        }
    }

    #[tokio::test]
    async fn test_resolve_bearer_token_if_available_uses_runtime_env_override() {
        let _guard = ENV_MUTEX.lock().expect("env mutex");
        let prev = std::env::var("NEARAI_API_KEY").ok();
        // SAFETY: tests hold ENV_MUTEX while mutating the process environment.
        unsafe { std::env::remove_var("NEARAI_API_KEY") };
        let _env_guard = EnvGuard("NEARAI_API_KEY", prev);

        set_runtime_env("NEARAI_API_KEY", "runtime-overlay-key");
        let session = SessionManager::new(SessionConfig::default());

        let token = resolve_nearai_bearer_token_if_available(None, &session)
            .await
            .expect("resolve token");

        assert_eq!(token.as_deref(), Some("runtime-overlay-key"));
    }
}
