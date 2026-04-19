//! Centralized authentication manager for engine v2.
//!
//! Owns the pre-flight credential check logic and setup instruction lookup.
//! Replaces scattered auth knowledge across router.rs, effect_adapter.rs,
//! and extension_tools.rs with a single state machine.
//!
//! Three detection paths:
//! 1. **HTTP tool** — `SharedCredentialRegistry` + `SecretsStore::exists()`
//! 2. **WASM tools** — same path (WASM tools register host→credential mappings)
//! 3. **Extensions** — `ExtensionManager::check_tool_auth_status()`

use std::sync::Arc;

use crate::extensions::naming::canonicalize_extension_name;
use crate::secrets::SecretsStore;
use crate::tools::builtin::extract_host_from_params;
use crate::tools::wasm::SharedCredentialRegistry;
use ironclaw_skills::{SkillCredentialSpec, SkillRegistry};

/// Result of checking whether a tool call has the credentials it needs.
#[derive(Debug)]
pub enum AuthCheckResult {
    /// Credentials are present — proceed with execution.
    Ready,
    /// Tool does not require any credentials for this call.
    NoAuthRequired,
    /// One or more credentials are missing — pause and prompt.
    MissingCredentials(Vec<MissingCredential>),
}

/// A single missing credential identified during pre-flight check.
#[derive(Debug, Clone)]
pub struct MissingCredential {
    /// Secret name in the secrets store (e.g., "github_token").
    pub credential_name: String,
    /// Human-readable setup instructions from the skill spec.
    pub setup_instructions: Option<String>,
    /// Optional OAuth URL that should be opened in the browser.
    pub auth_url: Option<String>,
}

/// Higher-level tool readiness for `available_actions()` filtering.
#[derive(Debug)]
pub enum ToolReadiness {
    /// Tool is ready to use.
    Ready,
    /// Tool needs auth (OAuth or manual token) before it can work.
    NeedsAuth {
        credential_name: String,
        instructions: Option<String>,
        auth_url: Option<String>,
    },
    /// Tool needs admin setup (client_id/secret) — cannot be resolved in chat.
    NeedsSetup { message: String },
}

/// Centralized auth state for the engine v2 bridge layer.
///
/// Provides pre-flight credential checking, setup instruction lookup,
/// and tool readiness queries. Injected into `EffectBridgeAdapter` and
/// `EngineState` by the router at init time.
pub struct AuthManager {
    secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
    extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
}

impl AuthManager {
    pub fn new(
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
        skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
        extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
    ) -> Self {
        Self {
            secrets_store,
            skill_registry,
            extension_manager,
        }
    }

    /// Pre-flight credential check for a tool call.
    ///
    /// For the `http` tool (and WASM tools that use the same credential
    /// injection path), extracts the target host from params, looks up
    /// registered credential mappings, and checks whether the required
    /// secrets exist in the store.
    pub async fn check_action_auth(
        &self,
        action_name: &str,
        parameters: &serde_json::Value,
        user_id: &str,
        credential_registry: &SharedCredentialRegistry,
    ) -> AuthCheckResult {
        let is_http = action_name == "http" || action_name == "http_request";

        if is_http {
            return self
                .check_http_auth(parameters, user_id, credential_registry)
                .await;
        }

        // For non-HTTP tools, we don't have a generic pre-flight mechanism
        // yet. Extension-level auth (NeedsAuth/NeedsSetup) is handled by
        // check_tool_readiness() for available_actions() filtering and by
        // the post-install pipeline.
        AuthCheckResult::NoAuthRequired
    }

    /// Check HTTP tool credentials by extracting the host and querying
    /// the credential registry + secrets store.
    async fn check_http_auth(
        &self,
        parameters: &serde_json::Value,
        user_id: &str,
        credential_registry: &SharedCredentialRegistry,
    ) -> AuthCheckResult {
        let host = match extract_host_from_params(parameters) {
            Some(h) => h,
            None => {
                tracing::debug!("Pre-flight auth: no host in params — skipping");
                return AuthCheckResult::NoAuthRequired;
            }
        };

        let matched = credential_registry.find_for_host(&host);
        tracing::debug!(
            host = %host,
            matched_count = matched.len(),
            "Pre-flight auth: credential registry lookup"
        );
        if matched.is_empty() {
            return AuthCheckResult::NoAuthRequired;
        }

        let mut missing = Vec::new();
        for mapping in &matched {
            match self
                .secrets_store
                .exists(user_id, &mapping.secret_name)
                .await
            {
                Ok(true) => {
                    // At least one credential is configured — tool can proceed.
                    // (Multiple mappings for the same host is normal, e.g.,
                    // Bearer token + org header. If any is present, we allow
                    // execution and let the HTTP tool handle partial injection.)
                    return AuthCheckResult::Ready;
                }
                Ok(false) => {
                    missing.push(
                        self.describe_missing_credential(&mapping.secret_name, user_id)
                            .await,
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        secret = %mapping.secret_name,
                        error = %e,
                        "Failed to check credential existence — assuming missing"
                    );
                    missing.push(MissingCredential {
                        credential_name: mapping.secret_name.clone(),
                        setup_instructions: None,
                        auth_url: None,
                    });
                }
            }
        }

        if missing.is_empty() {
            AuthCheckResult::Ready
        } else {
            AuthCheckResult::MissingCredentials(missing)
        }
    }

    /// Check whether a tool (by name) is ready to use, needs auth, or
    /// needs admin setup. Used by `available_actions()` to filter tools
    /// that cannot function at all.
    ///
    /// For installed extensions, this uses the extension manager's canonical
    /// `auth()` entrypoint so MCP and WASM tools share the same OAuth/manual
    /// auth behavior. This may initiate an OAuth flow and return the hosted
    /// authorization URL.
    pub async fn check_tool_readiness(&self, tool_name: &str, user_id: &str) -> ToolReadiness {
        let ext_mgr = match self.extension_manager.as_ref() {
            Some(mgr) => mgr,
            None => return ToolReadiness::Ready,
        };

        let ext_name = match canonicalize_extension_name(tool_name) {
            Ok(name) => name,
            Err(_) => return ToolReadiness::Ready,
        };
        match ext_mgr.auth(&ext_name, user_id).await {
            Ok(auth_result) => match auth_result.status {
                crate::extensions::AuthStatus::Authenticated
                | crate::extensions::AuthStatus::NoAuthRequired => ToolReadiness::Ready,
                crate::extensions::AuthStatus::AwaitingAuthorization { auth_url, .. } => {
                    let credential_name = ext_mgr
                        .first_missing_auth_secret_pub(&ext_name, user_id)
                        .await
                        .unwrap_or_else(|| ext_name.clone());
                    ToolReadiness::NeedsAuth {
                        credential_name,
                        instructions: Some(format!(
                            "Authenticate '{}' to finish setup.",
                            auth_result.name
                        )),
                        auth_url: Some(auth_url),
                    }
                }
                crate::extensions::AuthStatus::AwaitingToken { instructions, .. } => {
                    if let Some(secret_name) = ext_mgr
                        .first_missing_auth_secret_pub(&ext_name, user_id)
                        .await
                    {
                        let described = self
                            .describe_missing_credential(&secret_name, user_id)
                            .await;
                        ToolReadiness::NeedsAuth {
                            credential_name: described.credential_name,
                            instructions: described.setup_instructions.or(Some(instructions)),
                            auth_url: described.auth_url,
                        }
                    } else {
                        ToolReadiness::NeedsAuth {
                            credential_name: ext_name.clone(),
                            instructions: Some(instructions),
                            auth_url: None,
                        }
                    }
                }
                crate::extensions::AuthStatus::NeedsSetup { instructions, .. } => {
                    ToolReadiness::NeedsSetup {
                        message: instructions,
                    }
                }
            },
            Err(e) => {
                tracing::debug!(
                    tool = %ext_name,
                    user_id = %user_id,
                    error = %e,
                    "Extension auth readiness probe failed; treating tool as ready"
                );
                ToolReadiness::Ready
            }
        }
    }

    async fn describe_missing_credential(
        &self,
        credential_name: &str,
        user_id: &str,
    ) -> MissingCredential {
        let setup_instructions = self.get_setup_instructions(credential_name);
        let auth_url = self
            .start_skill_oauth_if_supported(credential_name, user_id)
            .await;
        let setup_instructions = if auth_url.is_some() {
            Some(
                setup_instructions
                    .unwrap_or_else(|| format!("Authenticate '{}' to continue.", credential_name)),
            )
        } else {
            setup_instructions
        };

        MissingCredential {
            credential_name: credential_name.to_string(),
            setup_instructions,
            auth_url,
        }
    }

    async fn start_skill_oauth_if_supported(
        &self,
        credential_name: &str,
        user_id: &str,
    ) -> Option<String> {
        use crate::cli::oauth_defaults;

        let spec = self.get_credential_spec(credential_name)?;
        let oauth = spec.oauth.as_ref()?;
        let builtin = oauth_defaults::builtin_credentials(credential_name)?;
        let ext_mgr = self.extension_manager.as_ref()?;
        let use_gateway = oauth_defaults::use_gateway_callback();
        let redirect_uri = if use_gateway {
            oauth_defaults::callback_url()
        } else {
            format!("{}/callback", oauth_defaults::callback_url())
        };
        let oauth_result = oauth_defaults::build_oauth_url(
            &oauth.authorization_url,
            builtin.client_id,
            &redirect_uri,
            &oauth.scopes,
            oauth.use_pkce,
            &oauth.extra_params,
        );

        let validation_endpoint =
            oauth
                .test_url
                .as_ref()
                .map(|url| crate::tools::wasm::ValidationEndpointSchema {
                    url: url.clone(),
                    method: "GET".to_string(),
                    success_status: 200,
                    headers: std::collections::HashMap::new(),
                });

        let pending_flow = oauth_defaults::PendingOAuthFlow {
            extension_name: credential_name.to_string(),
            display_name: spec.provider.clone(),
            token_url: oauth.token_url.clone(),
            client_id: builtin.client_id.to_string(),
            client_secret: Some(builtin.client_secret.to_string()),
            redirect_uri: redirect_uri.clone(),
            code_verifier: oauth_result.code_verifier.clone(),
            access_token_field: "access_token".to_string(),
            secret_name: credential_name.to_string(),
            provider: Some(spec.provider.clone()),
            validation_endpoint: validation_endpoint.clone(),
            scopes: oauth.scopes.clone(),
            user_id: user_id.to_string(),
            secrets: Arc::clone(&self.secrets_store),
            sse_manager: ext_mgr.sse_sender().await,
            gateway_token: oauth_defaults::oauth_proxy_auth_token(),
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at: std::time::Instant::now(),
            auto_activate_extension: false,
        };

        if use_gateway {
            let mut pending_flows = ext_mgr.pending_oauth_flows().write().await;
            pending_flows.retain(|_, flow| {
                !(flow.secret_name == credential_name && flow.user_id == user_id)
            });
            pending_flows.insert(oauth_result.state.clone(), pending_flow);
        } else {
            let listener = oauth_defaults::bind_callback_listener().await.ok()?;
            let display_name = pending_flow.display_name.clone();
            let token_url = pending_flow.token_url.clone();
            let client_id = pending_flow.client_id.clone();
            let client_secret = pending_flow.client_secret.clone();
            let code_verifier = pending_flow.code_verifier.clone();
            let access_token_field = pending_flow.access_token_field.clone();
            let provider = pending_flow.provider.clone();
            let scopes = pending_flow.scopes.clone();
            let secret_name = pending_flow.secret_name.clone();
            let validation_endpoint = pending_flow.validation_endpoint.clone();
            let secrets = Arc::clone(&pending_flow.secrets);
            let user_id = pending_flow.user_id.clone();
            let expected_state = oauth_result.state.clone();
            tokio::spawn(async move {
                let result: Result<(), String> = async {
                    let code = oauth_defaults::wait_for_callback(
                        listener,
                        "/callback",
                        "code",
                        &display_name,
                        Some(&expected_state),
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                    let token_response = oauth_defaults::exchange_oauth_code(
                        &token_url,
                        &client_id,
                        client_secret.as_deref(),
                        &code,
                        &redirect_uri,
                        code_verifier.as_deref(),
                        &access_token_field,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                    if let Some(ref validation) = validation_endpoint {
                        oauth_defaults::validate_oauth_token(
                            &token_response.access_token,
                            validation,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    }

                    oauth_defaults::store_oauth_tokens(
                        secrets.as_ref(),
                        &user_id,
                        &secret_name,
                        provider.as_deref(),
                        &token_response.access_token,
                        token_response.refresh_token.as_deref(),
                        token_response.expires_in,
                        &scopes,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    Ok(())
                }
                .await;

                if let Err(error) = result {
                    tracing::warn!(
                        credential = %secret_name,
                        user_id = %user_id,
                        error = %error,
                        "Local generic OAuth flow failed"
                    );
                    return;
                }

                if let Err(error) =
                    crate::bridge::resolve_engine_auth_callback(&user_id, &secret_name).await
                {
                    tracing::warn!(
                        credential = %secret_name,
                        user_id = %user_id,
                        error = %error,
                        "Failed to resume pending auth gate after local OAuth callback"
                    );
                }
            });
        }

        Some(oauth_result.url)
    }

    fn get_credential_spec(&self, credential_name: &str) -> Option<SkillCredentialSpec> {
        self.skill_registry.as_ref().and_then(|sr| {
            let reg = sr.read().ok()?;
            reg.skills().iter().find_map(|s| {
                s.manifest.credentials.iter().find_map(|c| {
                    if c.name == credential_name {
                        Some(c.clone())
                    } else {
                        None
                    }
                })
            })
        })
    }

    /// Look up human-readable setup instructions for a credential.
    ///
    /// Checks the skill registry for matching credential specs with
    /// `setup_instructions`. Falls back to a generic prompt.
    pub fn get_setup_instructions(&self, credential_name: &str) -> Option<String> {
        self.skill_registry.as_ref().and_then(|sr| {
            let reg = sr.read().ok()?;
            reg.skills().iter().find_map(|s| {
                s.manifest.credentials.iter().find_map(|c| {
                    if c.name == credential_name {
                        c.setup_instructions.clone()
                    } else {
                        None
                    }
                })
            })
        })
    }

    /// Get setup instructions with a fallback default message.
    pub fn get_setup_instructions_or_default(&self, credential_name: &str) -> String {
        self.get_setup_instructions(credential_name)
            .unwrap_or_else(|| format!("Provide your {} token", credential_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::credentials::test_secrets_store;
    use crate::tools::ToolRegistry;
    use std::path::Path;

    fn make_registry_with_mapping(secret_name: &str, host: &str) -> SharedCredentialRegistry {
        use crate::secrets::CredentialMapping;
        let registry = SharedCredentialRegistry::new();
        registry.add_mappings(vec![CredentialMapping::bearer(secret_name, host)]);
        registry
    }

    fn make_auth_manager(secrets_store: Arc<dyn SecretsStore + Send + Sync>) -> AuthManager {
        AuthManager::new(secrets_store, None, None)
    }

    fn make_extension_manager(
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
        wasm_tools_dir: &Path,
        wasm_channels_dir: &Path,
    ) -> Arc<crate::extensions::ExtensionManager> {
        Arc::new(crate::extensions::ExtensionManager::new(
            Arc::new(crate::tools::mcp::session::McpSessionManager::new()),
            Arc::new(crate::tools::mcp::process::McpProcessManager::new()),
            secrets_store,
            Arc::new(ToolRegistry::new()),
            None,
            None,
            wasm_tools_dir.to_path_buf(),
            wasm_channels_dir.to_path_buf(),
            None,
            "test-user".to_string(),
            None,
            vec![],
        ))
    }

    async fn make_skill_registry_with_google_oauth(
        dir: &Path,
    ) -> Arc<std::sync::RwLock<ironclaw_skills::SkillRegistry>> {
        std::fs::create_dir_all(dir.join("gmail-skill")).expect("create skill dir");
        std::fs::write(
            dir.join("gmail-skill").join("SKILL.md"),
            r#"---
name: gmail
version: "1.0.0"
description: Gmail OAuth test
activation:
  keywords: ["gmail"]
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts: ["gmail.googleapis.com"]
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      scopes: ["https://www.googleapis.com/auth/gmail.modify"]
      test_url: "https://www.googleapis.com/oauth2/v1/userinfo"
    setup_instructions: "Sign in with Google"
---
Test skill
"#,
        )
        .expect("write skill");

        let mut registry = ironclaw_skills::SkillRegistry::new(dir.to_path_buf());
        registry.discover_all().await;
        Arc::new(std::sync::RwLock::new(registry))
    }

    fn test_store() -> Arc<dyn SecretsStore + Send + Sync> {
        Arc::new(test_secrets_store())
    }

    struct TestEnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl Drop for TestEnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                // SAFETY: tests serialize env mutation with lock_env().
                unsafe { std::env::set_var(self.key, value) };
            } else {
                // SAFETY: tests serialize env mutation with lock_env().
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    fn set_test_env_var(key: &'static str, value: Option<&str>) -> TestEnvVarGuard {
        let original = std::env::var(key).ok();
        match value {
            // SAFETY: tests serialize env mutation with lock_env().
            Some(value) => unsafe { std::env::set_var(key, value) },
            // SAFETY: tests serialize env mutation with lock_env().
            None => unsafe { std::env::remove_var(key) },
        }
        TestEnvVarGuard { key, original }
    }

    #[tokio::test]
    async fn check_http_missing_credential() {
        let store = test_store();
        let mgr = make_auth_manager(store);
        let registry = make_registry_with_mapping("github_token", "api.github.com");

        let params = serde_json::json!({"url": "https://api.github.com/repos"});
        let result = mgr
            .check_action_auth("http", &params, "user1", &registry)
            .await;

        assert!(
            matches!(result, AuthCheckResult::MissingCredentials(ref m) if m.len() == 1),
            "Expected MissingCredentials, got {result:?}"
        );
        if let AuthCheckResult::MissingCredentials(missing) = result {
            assert_eq!(missing[0].credential_name, "github_token");
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn check_http_missing_credential_starts_skill_oauth_flow() {
        let _env_guard = crate::config::helpers::lock_env();
        let _callback_guard = set_test_env_var(
            "IRONCLAW_OAUTH_CALLBACK_URL",
            Some("https://example.com/oauth/callback"),
        );

        let store = test_store();
        let skills_dir = tempfile::tempdir().expect("skills dir");
        let skill_registry = make_skill_registry_with_google_oauth(skills_dir.path()).await;
        let wasm_tools_dir = tempfile::tempdir().expect("wasm tools dir");
        let wasm_channels_dir = tempfile::tempdir().expect("wasm channels dir");
        let ext_mgr = make_extension_manager(
            Arc::clone(&store),
            wasm_tools_dir.path(),
            wasm_channels_dir.path(),
        );
        let mgr = AuthManager::new(
            Arc::clone(&store),
            Some(skill_registry),
            Some(Arc::clone(&ext_mgr)),
        );
        let registry = make_registry_with_mapping("google_oauth_token", "gmail.googleapis.com");
        let params =
            serde_json::json!({"url": "https://gmail.googleapis.com/gmail/v1/users/me/profile"});

        let first = mgr
            .check_action_auth("http", &params, "user1", &registry)
            .await;
        let AuthCheckResult::MissingCredentials(first_missing) = first else {
            panic!("expected missing credential");
        };
        assert_eq!(first_missing.len(), 1);
        assert_eq!(first_missing[0].credential_name, "google_oauth_token");
        assert_eq!(
            first_missing[0].setup_instructions.as_deref(),
            Some("Sign in with Google")
        );
        let auth_url = first_missing[0].auth_url.as_ref().expect("oauth auth url");
        assert!(auth_url.contains("accounts.google.com"));

        let flows = ext_mgr.pending_oauth_flows().read().await;
        assert_eq!(flows.len(), 1);
        let flow = flows.values().next().expect("pending oauth flow");
        assert_eq!(flow.secret_name, "google_oauth_token");
        assert!(!flow.auto_activate_extension);
        drop(flows);

        let second = mgr
            .check_action_auth("http", &params, "user1", &registry)
            .await;
        let AuthCheckResult::MissingCredentials(second_missing) = second else {
            panic!("expected missing credential on retry");
        };
        assert!(second_missing[0].auth_url.is_some());
        assert_eq!(ext_mgr.pending_oauth_flows().read().await.len(), 1);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn check_wasm_channel_readiness_uses_secret_oauth_metadata() {
        let _env_guard = crate::config::helpers::lock_env();
        let _callback_guard = set_test_env_var(
            "IRONCLAW_OAUTH_CALLBACK_URL",
            Some("https://example.com/oauth/callback"),
        );

        let store = test_store();
        let skills_dir = tempfile::tempdir().expect("skills dir");
        let skill_registry = make_skill_registry_with_google_oauth(skills_dir.path()).await;
        let wasm_tools_dir = tempfile::tempdir().expect("wasm tools dir");
        let wasm_channels_dir = tempfile::tempdir().expect("wasm channels dir");
        std::fs::write(
            wasm_channels_dir.path().join("gmail_channel.wasm"),
            b"fake-channel",
        )
        .expect("write channel wasm");
        std::fs::write(
            wasm_channels_dir
                .path()
                .join("gmail_channel.capabilities.json"),
            serde_json::json!({
                "name": "gmail_channel",
                "description": "gmail test channel",
                "setup": {
                    "required_secrets": [
                        {"name": "google_oauth_token", "prompt": "Google OAuth token"}
                    ],
                    "setup_url": "https://example.com/setup"
                }
            })
            .to_string(),
        )
        .expect("write channel caps");
        let ext_mgr = make_extension_manager(
            Arc::clone(&store),
            wasm_tools_dir.path(),
            wasm_channels_dir.path(),
        );
        let mgr = AuthManager::new(store, Some(skill_registry), Some(ext_mgr));

        let readiness = mgr.check_tool_readiness("gmail-channel", "user1").await;
        match readiness {
            ToolReadiness::NeedsAuth {
                credential_name,
                instructions,
                auth_url,
            } => {
                assert_eq!(credential_name, "google_oauth_token");
                assert_eq!(instructions.as_deref(), Some("Sign in with Google"));
                assert!(
                    auth_url
                        .as_deref()
                        .is_some_and(|url| url.contains("accounts.google.com"))
                );
            }
            other => panic!("expected auth gate for wasm channel, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_http_credential_present() {
        let store = test_store();
        // Store a credential
        let params = crate::secrets::CreateSecretParams::new("github_token", "ghp_test123");
        store.create("user1", params).await.unwrap();

        let mgr = make_auth_manager(store);
        let registry = make_registry_with_mapping("github_token", "api.github.com");

        let params = serde_json::json!({"url": "https://api.github.com/repos"});
        let result = mgr
            .check_action_auth("http", &params, "user1", &registry)
            .await;

        assert!(
            matches!(result, AuthCheckResult::Ready),
            "Expected Ready, got {result:?}"
        );
    }

    #[tokio::test]
    async fn check_http_no_credential_mapping() {
        let store = test_store();
        let mgr = make_auth_manager(store);
        let registry = SharedCredentialRegistry::new(); // empty

        let params = serde_json::json!({"url": "https://httpbin.org/get"});
        let result = mgr
            .check_action_auth("http", &params, "user1", &registry)
            .await;

        assert!(
            matches!(result, AuthCheckResult::NoAuthRequired),
            "Expected NoAuthRequired, got {result:?}"
        );
    }

    #[tokio::test]
    async fn check_http_no_url_param() {
        let store = test_store();
        let mgr = make_auth_manager(store);
        let registry = make_registry_with_mapping("token", "api.example.com");

        let params = serde_json::json!({"method": "GET"});
        let result = mgr
            .check_action_auth("http", &params, "user1", &registry)
            .await;

        assert!(
            matches!(result, AuthCheckResult::NoAuthRequired),
            "Expected NoAuthRequired when no URL, got {result:?}"
        );
    }

    #[tokio::test]
    async fn check_non_http_tool_returns_no_auth_required() {
        let store = test_store();
        let mgr = make_auth_manager(store);
        let registry = make_registry_with_mapping("token", "api.example.com");

        let params = serde_json::json!({"query": "test"});
        let result = mgr
            .check_action_auth("echo", &params, "user1", &registry)
            .await;

        assert!(
            matches!(result, AuthCheckResult::NoAuthRequired),
            "Expected NoAuthRequired for non-HTTP tool, got {result:?}"
        );
    }

    #[tokio::test]
    async fn check_http_underscore_name_variant() {
        let store = test_store();
        let mgr = make_auth_manager(store);
        let registry = make_registry_with_mapping("api_key", "api.openai.com");

        let params = serde_json::json!({"url": "https://api.openai.com/v1/chat"});
        let result = mgr
            .check_action_auth("http_request", &params, "user1", &registry)
            .await;

        assert!(
            matches!(result, AuthCheckResult::MissingCredentials(_)),
            "Expected MissingCredentials for http_request variant, got {result:?}"
        );
    }

    #[test]
    fn get_setup_instructions_returns_none_without_skill_registry() {
        let store = test_store();
        let mgr = make_auth_manager(store);

        assert!(mgr.get_setup_instructions("github_token").is_none());
    }

    #[test]
    fn get_setup_instructions_or_default_returns_fallback() {
        let store = test_store();
        let mgr = make_auth_manager(store);

        let result = mgr.get_setup_instructions_or_default("github_token");
        assert_eq!(result, "Provide your github_token token");
    }
}
