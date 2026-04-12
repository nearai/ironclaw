//! Centralized authentication manager for engine v2.
//!
//! Owns the pre-flight credential check logic and setup instruction lookup.
//! Replaces scattered auth knowledge across router.rs, effect_adapter.rs,
//! and extension_tools.rs with a single state machine.
//!
//! Three detection paths:
//! 1. **HTTP tool** — `SharedCredentialRegistry` + shared refresh-aware credential resolution
//! 2. **WASM tools** — same path (WASM tools register host→credential mappings)
//! 3. **Extensions** — `ExtensionManager::check_tool_auth_status()`

use std::sync::Arc;

use crate::auth::{
    AuthDescriptor, AuthDescriptorKind, DefaultFallback, OAuthFlowDescriptor,
    PendingOAuthLaunchParams, build_pending_oauth_launch, resolve_secret_for_runtime,
    upsert_auth_descriptor,
};
use crate::extensions::naming::canonicalize_extension_name;
use crate::extensions::{ConfigureResult, ExtensionError};
use crate::secrets::SecretsStore;
use crate::tools::ToolRegistry;
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

#[derive(Debug, Clone)]
pub struct LatentActionDef {
    pub action_name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
}

#[derive(Debug)]
pub enum LatentActionExecution {
    RetryRegisteredAction {
        resolved_action: String,
    },
    ProviderReady {
        provider_extension: String,
        available_actions: Vec<String>,
    },
    NeedsAuth {
        credential_name: String,
        instructions: String,
        auth_url: Option<String>,
    },
    NeedsSetup {
        message: String,
    },
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
    tools: Option<Arc<ToolRegistry>>,
}

impl AuthManager {
    pub fn new(
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
        skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
        extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
        tools: Option<Arc<ToolRegistry>>,
    ) -> Self {
        Self {
            secrets_store,
            skill_registry,
            extension_manager,
            tools,
        }
    }

    fn settings_store(&self) -> Option<&dyn crate::db::SettingsStore> {
        self.tools
            .as_ref()
            .and_then(|tools| {
                tools
                    .database()
                    .map(|db| db.as_ref() as &dyn crate::db::SettingsStore)
            })
            .or_else(|| {
                self.extension_manager.as_ref().and_then(|manager| {
                    manager
                        .database()
                        .map(|db| db.as_ref() as &dyn crate::db::SettingsStore)
                })
            })
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
                tracing::debug!("Pre-flight auth: no host in params — skipping"); // safety: no actual param values logged
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
            let oauth_refresh = credential_registry.oauth_refresh_for_secret(&mapping.secret_name);
            let role_lookup = self
                .tools
                .as_ref()
                .and_then(|tools| tools.role_lookup().map(Arc::as_ref));
            match resolve_secret_for_runtime(
                self.secrets_store.as_ref(),
                user_id,
                &mapping.secret_name,
                role_lookup,
                oauth_refresh.as_ref(),
                DefaultFallback::AdminOnly,
            )
            .await
            {
                Ok(_) => {
                    // At least one credential is configured — tool can proceed.
                    // (Multiple mappings for the same host is normal, e.g.,
                    // Bearer token + org header. If any is present, we allow
                    // execution and let the HTTP tool handle partial injection.)
                    return AuthCheckResult::Ready;
                }
                Err(error) if error.requires_authentication() => {
                    missing.push(
                        self.describe_missing_credential(&mapping.secret_name, user_id)
                            .await,
                    );
                }
                Err(error) => {
                    tracing::debug!(
                        secret = %mapping.secret_name,
                        error = ?error,
                        "Failed to resolve credential during pre-flight auth — assuming missing"
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

        let ext_name = if let Some(tools) = self.tools.as_ref() {
            if let Some(name) = tools.provider_extension_for_tool(tool_name).await {
                name
            } else {
                match canonicalize_extension_name(tool_name) {
                    Ok(name) => name,
                    Err(_) => return ToolReadiness::Ready,
                }
            }
        } else {
            match canonicalize_extension_name(tool_name) {
                Ok(name) => name,
                Err(_) => return ToolReadiness::Ready,
            }
        };
        self
            .readiness_from_extension_result(
                &ext_name,
                user_id,
                ext_mgr
                    .ensure_extension_ready(
                        &ext_name,
                        user_id,
                        crate::extensions::EnsureReadyIntent::UseCapability,
                    )
                    .await,
            )
            .await
    }

    /// Prepare an extension-backed capability for immediate execution.
    ///
    /// Unlike [`check_tool_readiness`], this path may promote a latent
    /// registry-backed extension into the installed state because the caller
    /// is handling a concrete user-requested action, not merely listing or
    /// filtering available actions.
    pub async fn prepare_tool_for_execution(&self, tool_name: &str, user_id: &str) -> ToolReadiness {
        let ext_mgr = match self.extension_manager.as_ref() {
            Some(mgr) => mgr,
            None => return ToolReadiness::Ready,
        };

        let ext_name = if let Some(tools) = self.tools.as_ref() {
            if let Some(name) = tools.provider_extension_for_tool(tool_name).await {
                name
            } else {
                match canonicalize_extension_name(tool_name) {
                    Ok(name) => name,
                    Err(_) => return ToolReadiness::Ready,
                }
            }
        } else {
            match canonicalize_extension_name(tool_name) {
                Ok(name) => name,
                Err(_) => return ToolReadiness::Ready,
            }
        };

        let initial = ext_mgr
            .ensure_extension_ready(
                &ext_name,
                user_id,
                crate::extensions::EnsureReadyIntent::UseCapability,
            )
            .await;

        let result = match initial {
            Err(ExtensionError::NotInstalled(_)) => {
                tracing::debug!(
                    extension = %ext_name,
                    user_id = %user_id,
                    "Extension not installed for capability use; retrying via explicit activate path"
                );
                ext_mgr
                    .ensure_extension_ready(
                        &ext_name,
                        user_id,
                        crate::extensions::EnsureReadyIntent::ExplicitActivate,
                    )
                    .await
            }
            other => other,
        };

        self.readiness_from_extension_result(&ext_name, user_id, result)
            .await
    }

    async fn readiness_from_extension_result(
        &self,
        ext_name: &str,
        user_id: &str,
        result: Result<crate::extensions::EnsureReadyOutcome, ExtensionError>,
    ) -> ToolReadiness {
        match result {
            Ok(crate::extensions::EnsureReadyOutcome::Ready { .. }) => ToolReadiness::Ready,
            Ok(crate::extensions::EnsureReadyOutcome::NeedsAuth {
                auth,
                credential_name,
                ..
            }) => {
                let credential_name = credential_name.unwrap_or_else(|| ext_name.to_string());
                let described = self
                    .describe_missing_credential(&credential_name, user_id)
                    .await;
                let instructions = match &auth.status {
                    crate::extensions::AuthStatus::AwaitingAuthorization { .. } => described
                        .setup_instructions
                        .or_else(|| Some(format!("Authenticate '{}' to finish setup.", auth.name))),
                    crate::extensions::AuthStatus::AwaitingToken { instructions, .. } => {
                        described.setup_instructions.or(Some(instructions.clone()))
                    }
                    _ => described.setup_instructions,
                };
                ToolReadiness::NeedsAuth {
                    credential_name: described.credential_name,
                    instructions,
                    auth_url: crate::auth::oauth::sanitize_auth_url(auth.auth_url()).or_else(
                        || crate::auth::oauth::sanitize_auth_url(described.auth_url.as_deref()),
                    ),
                }
            }
            Ok(crate::extensions::EnsureReadyOutcome::NeedsSetup { instructions, .. }) => {
                ToolReadiness::NeedsSetup {
                    message: instructions,
                }
            }
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

    pub async fn latent_extension_actions(&self) -> Vec<LatentActionDef> {
        let Some(ext_mgr) = self.extension_manager.as_ref() else {
            return Vec::new();
        };

        ext_mgr
            .latent_provider_actions_default_user()
            .await
            .into_iter()
            .map(|action| LatentActionDef {
                action_name: action.action_name,
                description: action.description,
                parameters_schema: action.parameters_schema,
            })
            .collect()
    }

    pub async fn execute_latent_extension_action(
        &self,
        action_name: &str,
        user_id: &str,
    ) -> Option<Result<LatentActionExecution, crate::extensions::ExtensionError>> {
        let ext_mgr = self.extension_manager.as_ref()?;
        let latent = ext_mgr.latent_provider_action(action_name, user_id).await?;

        Some(
            match match ext_mgr
                .ensure_extension_ready(
                    &latent.provider_extension,
                    user_id,
                    crate::extensions::EnsureReadyIntent::UseCapability,
                )
                .await
            {
                Err(ExtensionError::NotInstalled(_)) => {
                    tracing::debug!(
                        extension = %latent.provider_extension,
                        user_id = %user_id,
                        "Latent action hit uninstalled provider; retrying via explicit activate path"
                    );
                    ext_mgr
                        .ensure_extension_ready(
                            &latent.provider_extension,
                            user_id,
                            crate::extensions::EnsureReadyIntent::ExplicitActivate,
                        )
                        .await
                }
                other => other,
            } {
                Ok(crate::extensions::EnsureReadyOutcome::Ready { .. }) => {
                    let available_actions = ext_mgr
                        .provider_action_names(&latent.provider_extension)
                        .await;
                    if available_actions.contains(&latent.action_name) {
                        Ok(LatentActionExecution::RetryRegisteredAction {
                            resolved_action: latent.action_name,
                        })
                    } else {
                        Ok(LatentActionExecution::ProviderReady {
                            provider_extension: latent.provider_extension,
                            available_actions,
                        })
                    }
                }
                Ok(crate::extensions::EnsureReadyOutcome::NeedsAuth {
                    auth,
                    credential_name,
                    ..
                }) => Ok(LatentActionExecution::NeedsAuth {
                    credential_name: credential_name.unwrap_or(latent.provider_extension),
                    instructions: auth
                        .instructions()
                        .unwrap_or("Complete authentication to continue.")
                        .to_string(),
                    auth_url: crate::auth::oauth::sanitize_auth_url(auth.auth_url()),
                }),
                Ok(crate::extensions::EnsureReadyOutcome::NeedsSetup { instructions, .. }) => {
                    Ok(LatentActionExecution::NeedsSetup {
                        message: instructions,
                    })
                }
                Err(err) => Err(err),
            },
        )
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

    pub async fn submit_auth_token(
        &self,
        extension_name: &str,
        token: &str,
        user_id: &str,
    ) -> Result<ConfigureResult, ExtensionError> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(ExtensionError::ValidationFailed(
                "Credential cannot be empty.".to_string(),
            ));
        }

        if let Some(ext_mgr) = self.extension_manager.as_ref() {
            match ext_mgr
                .configure_token(extension_name, trimmed, user_id)
                .await
            {
                Ok(result) => return Ok(result),
                Err(ExtensionError::NotInstalled(_)) => {}
                Err(other) if other.to_string().contains("not found") => {}
                Err(other) => return Err(other),
            }
        }

        let Some(spec) = self.get_credential_spec(extension_name) else {
            return Err(ExtensionError::NotInstalled(extension_name.to_string()));
        };

        // Defense in depth: only ever write under the registered credential
        // name from the skill manifest, never the caller-provided string.
        // `get_credential_spec` already filters by `c.name == extension_name`,
        // so this is a tautology today, but it locks the invariant against
        // future drift in the lookup logic.
        if spec.name != extension_name {
            return Err(ExtensionError::ValidationFailed(format!(
                "Credential name mismatch: requested '{}', resolved '{}'",
                extension_name, spec.name
            )));
        }

        let mut params = crate::secrets::CreateSecretParams::new(&spec.name, trimmed);
        if !spec.provider.is_empty() {
            params = params.with_provider(spec.provider.clone());
        }

        self.secrets_store
            .create(user_id, params)
            .await
            .map_err(|e| ExtensionError::Other(format!("Failed to store credential: {e}")))?;

        Ok(ConfigureResult {
            message: format!("Credential '{}' stored.", spec.name),
            activated: true,
            pairing_required: false,
            auth_url: None,
            verification: None,
            onboarding_state: None,
            onboarding: None,
        })
    }

    async fn start_skill_oauth_if_supported(
        &self,
        credential_name: &str,
        user_id: &str,
    ) -> Option<String> {
        use crate::auth::oauth;

        let spec = self.get_credential_spec(credential_name)?;
        let oauth = spec.oauth.as_ref()?;
        let descriptor = AuthDescriptor {
            kind: AuthDescriptorKind::SkillCredential,
            secret_name: spec.name.clone(),
            integration_name: spec.name.clone(),
            display_name: Some(spec.provider.clone()),
            provider: Some(spec.provider.clone()),
            setup_url: None,
            oauth: Some(OAuthFlowDescriptor {
                authorization_url: oauth.authorization_url.clone(),
                token_url: oauth.token_url.clone(),
                client_id: oauth.client_id.clone(),
                client_id_env: oauth.client_id_env.clone(),
                client_secret: oauth.client_secret.clone(),
                client_secret_env: oauth.client_secret_env.clone(),
                scopes: oauth.scopes.clone(),
                use_pkce: oauth.use_pkce,
                extra_params: oauth.extra_params.clone(),
                access_token_field: "access_token".to_string(),
                validation_url: oauth.test_url.clone(),
            }),
        };
        let builtin = oauth::builtin_credentials(credential_name);
        let exchange_proxy_url = oauth::exchange_proxy_url();
        let client_id = oauth
            .client_id
            .clone()
            .or_else(|| {
                oauth
                    .client_id_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            })
            .or_else(|| builtin.as_ref().map(|c| c.client_id.to_string()))?;
        let client_secret = oauth
            .client_secret
            .clone()
            .or_else(|| {
                oauth
                    .client_secret_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            })
            .or_else(|| builtin.as_ref().map(|c| c.client_secret.to_string()));
        let client_secret = oauth::hosted_proxy_client_secret(
            &client_secret,
            builtin.as_ref(),
            exchange_proxy_url.is_some(),
        );
        let ext_mgr = self.extension_manager.as_ref()?;
        upsert_auth_descriptor(self.settings_store(), user_id, descriptor).await;
        let use_gateway = oauth::use_gateway_callback();
        let redirect_uri = if use_gateway {
            oauth::callback_url()
        } else {
            format!("{}/callback", oauth::callback_url())
        };
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

        let launch = build_pending_oauth_launch(PendingOAuthLaunchParams {
            extension_name: credential_name.to_string(),
            display_name: spec.provider.clone(),
            authorization_url: oauth.authorization_url.clone(),
            token_url: oauth.token_url.clone(),
            client_id,
            client_secret,
            redirect_uri: redirect_uri.clone(),
            access_token_field: "access_token".to_string(),
            secret_name: credential_name.to_string(),
            provider: Some(spec.provider.clone()),
            validation_endpoint: validation_endpoint.clone(),
            scopes: oauth.scopes.clone(),
            use_pkce: oauth.use_pkce,
            extra_params: oauth.extra_params.clone(),
            user_id: user_id.to_string(),
            secrets: Arc::clone(&self.secrets_store),
            sse_manager: ext_mgr.sse_sender().await,
            gateway_token: oauth::oauth_proxy_auth_token(),
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            auto_activate_extension: false,
        });
        let pending_flow = launch.flow;

        if use_gateway {
            let auth_result = ext_mgr
                .start_hosted_oauth_flow(
                    credential_name.to_string(),
                    crate::extensions::ExtensionKind::WasmTool,
                    launch.auth_url.clone(),
                    launch.expected_state.clone(),
                    pending_flow,
                )
                .await;
            return auth_result.auth_url().map(ToString::to_string);
        } else {
            let listener = oauth::bind_callback_listener().await.ok()?;
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
            let expected_state = launch.expected_state.clone();
            tokio::spawn(async move {
                let result: Result<(), String> = async {
                    let code = oauth::wait_for_callback(
                        listener,
                        "/callback",
                        "code",
                        &display_name,
                        Some(&expected_state),
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                    let token_response = oauth::exchange_oauth_code(
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
                        oauth::validate_oauth_token(&token_response.access_token, validation)
                            .await
                            .map_err(|e| e.to_string())?;
                    }

                    oauth::store_oauth_tokens(
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

        Some(launch.auth_url)
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
        AuthManager::new(secrets_store, None, None, None)
    }

    fn make_extension_manager(
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
        wasm_tools_dir: &Path,
        wasm_channels_dir: &Path,
    ) -> Arc<crate::extensions::ExtensionManager> {
        make_extension_manager_with_registry(
            secrets_store,
            wasm_tools_dir,
            wasm_channels_dir,
            Arc::new(ToolRegistry::new()),
        )
    }

    fn make_extension_manager_with_registry(
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
        wasm_tools_dir: &Path,
        wasm_channels_dir: &Path,
        tools: Arc<ToolRegistry>,
    ) -> Arc<crate::extensions::ExtensionManager> {
        Arc::new(crate::extensions::ExtensionManager::new(
            Arc::new(crate::tools::mcp::session::McpSessionManager::new()),
            Arc::new(crate::tools::mcp::process::McpProcessManager::new()),
            secrets_store,
            tools,
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

    async fn make_skill_registry_with_custom_oauth(
        dir: &Path,
    ) -> Arc<std::sync::RwLock<ironclaw_skills::SkillRegistry>> {
        std::fs::create_dir_all(dir.join("custom-skill")).expect("create skill dir");
        std::fs::write(
            dir.join("custom-skill").join("SKILL.md"),
            r#"---
name: custom
version: "1.0.0"
description: Custom OAuth test
activation:
  keywords: ["custom"]
credentials:
  - name: custom_oauth_token
    provider: custom
    location:
      type: bearer
    hosts: ["api.custom.test"]
    oauth:
      authorization_url: "https://auth.custom.test/authorize"
      token_url: "https://auth.custom.test/token"
      client_id: "custom-client-id"
      client_secret: "custom-client-secret"
      scopes: ["read"]
    setup_instructions: "Sign in with Custom"
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
            None,
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
    #[allow(clippy::await_holding_lock)]
    async fn submit_auth_token_stores_declared_skill_credential() {
        let _env_guard = crate::config::helpers::lock_env();
        let store = test_store();
        let skills_dir = tempfile::tempdir().expect("skills dir");
        let skill_registry = make_skill_registry_with_google_oauth(skills_dir.path()).await;
        let mgr = AuthManager::new(Arc::clone(&store), Some(skill_registry), None, None);

        let result = mgr
            .submit_auth_token("google_oauth_token", "ya29.test-token", "user1")
            .await
            .expect("skill credential should store");

        assert!(result.activated);
        let stored = store
            .get_decrypted("user1", "google_oauth_token")
            .await
            .expect("stored secret");
        assert_eq!(stored.expose(), "ya29.test-token");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn submit_auth_token_rejects_unknown_credential_name() {
        // Regression: the skill-credential fallback path must only write
        // secrets whose name is declared by an installed skill manifest.
        // An unrecognized name must not result in a stored secret.
        let _env_guard = crate::config::helpers::lock_env();
        let store = test_store();
        let skills_dir = tempfile::tempdir().expect("skills dir");
        let skill_registry = make_skill_registry_with_google_oauth(skills_dir.path()).await;
        let mgr = AuthManager::new(Arc::clone(&store), Some(skill_registry), None, None);

        let result = mgr
            .submit_auth_token("not_a_declared_credential", "attacker-value", "user1")
            .await;
        assert!(
            matches!(result, Err(ExtensionError::NotInstalled(_))),
            "expected NotInstalled, got {:?}",
            result
        );
        // No secret should have been persisted under the attacker-supplied name.
        let stored = store
            .get_decrypted("user1", "not_a_declared_credential")
            .await;
        assert!(
            matches!(stored, Err(crate::secrets::SecretError::NotFound(_))),
            "no secret should be stored for an undeclared credential name, got {:?}",
            stored
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn check_http_missing_credential_starts_skill_oauth_flow_with_custom_client_config() {
        let _env_guard = crate::config::helpers::lock_env();
        let _callback_guard = set_test_env_var(
            "IRONCLAW_OAUTH_CALLBACK_URL",
            Some("https://example.com/oauth/callback"),
        );

        let store = test_store();
        let skills_dir = tempfile::tempdir().expect("skills dir");
        let skill_registry = make_skill_registry_with_custom_oauth(skills_dir.path()).await;
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
            None,
        );
        let registry = make_registry_with_mapping("custom_oauth_token", "api.custom.test");
        let params = serde_json::json!({"url": "https://api.custom.test/v1/me"});

        let result = mgr
            .check_action_auth("http", &params, "user1", &registry)
            .await;
        let AuthCheckResult::MissingCredentials(missing) = result else {
            panic!("expected missing credential");
        };
        let auth_url = missing[0].auth_url.as_ref().expect("oauth auth url");
        assert!(auth_url.contains("client_id=custom-client-id"));

        let flows = ext_mgr.pending_oauth_flows().read().await;
        let flow = flows.values().next().expect("pending oauth flow");
        assert_eq!(flow.client_id, "custom-client-id");
        assert_eq!(flow.client_secret.as_deref(), Some("custom-client-secret"));
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
            wasm_channels_dir.path().join("gmail-channel.wasm"),
            b"fake-channel",
        )
        .expect("write channel wasm");
        std::fs::write(
            wasm_channels_dir
                .path()
                .join("gmail-channel.capabilities.json"),
            serde_json::json!({
                "name": "gmail-channel",
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
        let credential_registry = crate::tools::wasm::SharedCredentialRegistry::new();
        {
            let guard = skill_registry.read().expect("skill registry");
            crate::skills::register_skill_credentials(guard.skills(), &credential_registry);
        }
        let tools = Arc::new(
            ToolRegistry::new().with_credentials(Arc::new(credential_registry), Arc::clone(&store)),
        );
        let ext_mgr = make_extension_manager_with_registry(
            Arc::clone(&store),
            wasm_tools_dir.path(),
            wasm_channels_dir.path(),
            Arc::clone(&tools),
        );
        let mgr = AuthManager::new(store, Some(skill_registry), Some(ext_mgr), Some(tools));

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
    #[allow(clippy::await_holding_lock)]
    async fn check_tool_readiness_resolves_action_to_provider_extension() {
        struct ProviderActionTool;

        #[async_trait::async_trait]
        impl crate::tools::Tool for ProviderActionTool {
            fn name(&self) -> &str {
                "gmail_send"
            }

            fn description(&self) -> &str {
                "gmail action"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            fn provider_extension(&self) -> Option<&str> {
                Some("gmail_channel")
            }

            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
                unreachable!()
            }
        }

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
                    ]
                }
            })
            .to_string(),
        )
        .expect("write channel caps");

        let credential_registry = crate::tools::wasm::SharedCredentialRegistry::new();
        {
            let guard = skill_registry.read().expect("skill registry");
            crate::skills::register_skill_credentials(guard.skills(), &credential_registry);
        }
        let tools = Arc::new(
            ToolRegistry::new().with_credentials(Arc::new(credential_registry), Arc::clone(&store)),
        );
        tools.register(Arc::new(ProviderActionTool)).await;
        let ext_mgr = make_extension_manager_with_registry(
            Arc::clone(&store),
            wasm_tools_dir.path(),
            wasm_channels_dir.path(),
            Arc::clone(&tools),
        );
        let mgr = AuthManager::new(store, Some(skill_registry), Some(ext_mgr), Some(tools));

        let readiness = mgr.check_tool_readiness("gmail_send", "user1").await;
        match readiness {
            ToolReadiness::NeedsAuth {
                credential_name,
                auth_url,
                ..
            } => {
                assert_eq!(credential_name, "google_oauth_token");
                assert!(auth_url.is_some());
            }
            other => panic!("expected provider-mapped auth gate, got {other:?}"),
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
    async fn check_http_expired_credential_without_refresh_is_missing() {
        let store = test_store();
        let params = crate::secrets::CreateSecretParams::new("github_token", "ghp_test123")
            .with_expiry(chrono::Utc::now() - chrono::Duration::hours(1));
        store.create("user1", params).await.unwrap();

        let mgr = make_auth_manager(store);
        let registry = make_registry_with_mapping("github_token", "api.github.com");

        let params = serde_json::json!({"url": "https://api.github.com/repos"});
        let result = mgr
            .check_action_auth("http", &params, "user1", &registry)
            .await;

        assert!(
            matches!(result, AuthCheckResult::MissingCredentials(ref m) if m.len() == 1),
            "Expected MissingCredentials for expired credential, got {result:?}"
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
