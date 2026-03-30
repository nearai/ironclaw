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

use crate::secrets::SecretsStore;
use crate::tools::builtin::extract_host_from_params;
use crate::tools::wasm::SharedCredentialRegistry;
use ironclaw_skills::SkillRegistry;

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
        let lookup_name = action_name.replace('_', "-");
        let is_http = action_name == "http"
            || action_name == "http_request"
            || lookup_name == "http"
            || lookup_name == "http-request";

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
                    let instructions = self.get_setup_instructions(&mapping.secret_name);
                    missing.push(MissingCredential {
                        credential_name: mapping.secret_name.clone(),
                        setup_instructions: instructions,
                    });
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
    /// Currently delegates to `ExtensionManager::check_tool_auth_status_pub()`
    /// for WASM extensions. Returns `Ready` for built-in tools and when
    /// no extension manager is available.
    pub async fn check_tool_readiness(&self, tool_name: &str, user_id: &str) -> ToolReadiness {
        let ext_mgr = match self.extension_manager.as_ref() {
            Some(mgr) => mgr,
            None => return ToolReadiness::Ready,
        };

        // Normalize tool name to extension name (tools are often
        // registered with hyphenated names that map to extensions).
        let ext_name = tool_name.replace('_', "-");

        let status = ext_mgr.check_tool_auth_status_pub(&ext_name, user_id).await;
        match status {
            crate::extensions::ToolAuthState::Ready | crate::extensions::ToolAuthState::NoAuth => {
                ToolReadiness::Ready
            }
            crate::extensions::ToolAuthState::NeedsAuth => {
                let instructions = self.get_setup_instructions(&ext_name);
                ToolReadiness::NeedsAuth {
                    credential_name: ext_name,
                    instructions,
                }
            }
            crate::extensions::ToolAuthState::NeedsSetup => ToolReadiness::NeedsSetup {
                message: format!(
                    "Extension '{}' needs to be configured in Settings before it can be used.",
                    ext_name
                ),
            },
        }
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

    fn make_registry_with_mapping(secret_name: &str, host: &str) -> SharedCredentialRegistry {
        use crate::secrets::CredentialMapping;
        let registry = SharedCredentialRegistry::new();
        registry.add_mappings(vec![CredentialMapping::bearer(secret_name, host)]);
        registry
    }

    fn make_auth_manager(secrets_store: Arc<dyn SecretsStore + Send + Sync>) -> AuthManager {
        AuthManager::new(secrets_store, None, None)
    }

    fn test_store() -> Arc<dyn SecretsStore + Send + Sync> {
        Arc::new(test_secrets_store())
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
