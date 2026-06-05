//! WebChat v2 auth-surface assembly for `ironclaw-reborn serve`.
//!
//! Owns the one place that turns host config into the pair the listener
//! needs: the `WebuiAuthenticator` the protected v2 routes use, plus the
//! optional public login-route mount. `serve.rs` only wires host config
//! and calls [`build_webui_auth_surface`]; it does not itself open the
//! user store, run the signed-session builder, or know the `Option`/
//! provider invariants — those live here, next to the admission adapter
//! ([`crate::commands::user_directory`]) and the startup config
//! ([`crate::commands::serve_sso`]).

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use ironclaw_reborn_composition::host_api::{AgentId, ProjectId, TenantId};
use ironclaw_reborn_composition::{
    LocalTriggerAccessReconciliation, LocalTriggerAccessRole, LocalTriggerAccessSource,
    PublicRouteMount, WebuiAuthenticator, open_local_trigger_access_store, open_webui_user_store,
};
use ironclaw_reborn_webui_ingress::{SignedSessionLoginConfig, build_signed_session_login};
use secrecy::SecretString;

use crate::commands::serve_sso::SsoStartupConfig;
use crate::commands::user_directory::{LocalTriggerAccessBootstrap, WebuiUserDirectory};

/// The composed WebChat v2 auth surface: the authenticator the protected
/// routes verify bearers with, plus the optional public login-route mount
/// (present only when SSO providers are configured).
pub(crate) struct WebuiAuthSurface {
    pub(crate) authenticator: Arc<dyn WebuiAuthenticator>,
    pub(crate) public_mount: Option<PublicRouteMount>,
}

pub(crate) struct LocalTriggerAccessBootstrapConfig {
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
    pub(crate) project_id: Option<ProjectId>,
}

/// Build the auth surface from resolved startup config.
///
/// With no SSO provider configured (`sso_startup` is `None`), the
/// listener keeps its plain env-bearer authenticator and mounts no public
/// routes. With providers configured, this opens the reborn-owned user
/// store on the substrate DB, layers the fail-closed email-domain
/// admission adapter on top, and hands the result to the ingress
/// signed-session builder.
pub(crate) async fn build_webui_auth_surface(
    sso_startup: Option<SsoStartupConfig>,
    user_store_path: &Path,
    tenant_id: TenantId,
    session_signing_secret: SecretString,
    env_authenticator: Arc<dyn WebuiAuthenticator>,
    local_trigger_access: Option<LocalTriggerAccessBootstrapConfig>,
) -> anyhow::Result<WebuiAuthSurface> {
    let Some(sso) = sso_startup else {
        if let Some(config) = local_trigger_access {
            reconcile_local_trigger_access(user_store_path, config, Vec::new())
                .await
                .context("failed to deactivate local trigger access for disabled SSO")?;
        }
        return Ok(WebuiAuthSurface {
            authenticator: env_authenticator,
            public_mount: None,
        });
    };

    // Open the reborn-owned user store through the composition facade
    // (which keeps the libSQL substrate handle private). The host
    // `WebuiUserDirectory` adapter layers the fail-closed email-domain
    // admission allowlist on top before any user is created.
    let user_store = open_webui_user_store(user_store_path)
        .await
        .context("failed to initialize WebChat user-identity store")?;
    let local_trigger_access = if let Some(config) = local_trigger_access {
        let admitted_user_ids = user_store
            .list_active_users_by_allowed_email_domains(&sso.allowed_email_domains)
            .await
            .context("failed to list admitted WebChat SSO users for local trigger access")?;
        Some(
            reconcile_local_trigger_access(user_store_path, config, admitted_user_ids)
                .await
                .context("failed to reconcile local trigger access for SSO users")?,
        )
    } else {
        None
    };
    let mut user_directory = WebuiUserDirectory::new(user_store, sso.allowed_email_domains);
    if let Some(local_trigger_access) = local_trigger_access {
        user_directory = user_directory.with_local_trigger_access(local_trigger_access);
    }

    let wiring = build_signed_session_login(SignedSessionLoginConfig {
        tenant_id,
        user_directory: Arc::new(user_directory),
        operator_secret: session_signing_secret,
        base_url: sso.base_url,
        providers: sso.providers,
        env_authenticator,
    })
    .expect("non-empty providers always produce login wiring"); // safety: sso_startup_config_from_env returns None when providers is empty, so this Some(sso) arm always has a non-empty provider list

    eprintln!(
        "ironclaw-reborn: WebChat v2 SSO login mounted — \
         see GET /auth/providers for the enabled set"
    );
    Ok(WebuiAuthSurface {
        authenticator: wiring.authenticator,
        public_mount: Some(wiring.mount),
    })
}

async fn reconcile_local_trigger_access(
    user_store_path: &Path,
    config: LocalTriggerAccessBootstrapConfig,
    admitted_user_ids: Vec<ironclaw_reborn_composition::host_api::UserId>,
) -> anyhow::Result<LocalTriggerAccessBootstrap> {
    let LocalTriggerAccessBootstrapConfig {
        tenant_id,
        agent_id,
        project_id,
    } = config;
    let access_store = open_local_trigger_access_store(user_store_path)
        .await
        .context("failed to initialize local trigger access store for SSO")?;
    access_store
        .reconcile_local_access(LocalTriggerAccessReconciliation {
            tenant_id: &tenant_id,
            user_ids: &admitted_user_ids,
            agent_id: Some(&agent_id),
            project_id: project_id.as_ref(),
            role: LocalTriggerAccessRole::Owner,
            source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
        })
        .await?;
    Ok(LocalTriggerAccessBootstrap::new(
        access_store,
        tenant_id,
        agent_id,
        project_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use ironclaw_reborn_composition::host_api::UserId;
    use ironclaw_reborn_composition::{ResolveIdentity, TriggerFireAccessChecker};
    use ironclaw_reborn_webui_ingress::{
        OAuthError, OAuthProvider, OAuthProviderName, OAuthUserProfile,
    };

    struct OneToken;

    #[async_trait]
    impl WebuiAuthenticator for OneToken {
        async fn authenticate(&self, token: &str) -> Option<UserId> {
            if token == "env-token" {
                Some(UserId::new("env-user").expect("user id"))
            } else {
                None
            }
        }
    }

    struct StubProvider(OAuthProviderName);

    #[async_trait]
    impl OAuthProvider for StubProvider {
        fn name(&self) -> &OAuthProviderName {
            &self.0
        }

        fn authorization_url(
            &self,
            _callback_url: &str,
            _state: &str,
            _code_challenge: &str,
        ) -> String {
            "https://provider.example/authorize".to_string()
        }

        async fn exchange_code(
            &self,
            _code: &str,
            _callback_url: &str,
            _code_verifier: &str,
        ) -> Result<OAuthUserProfile, OAuthError> {
            unreachable!("provider exchange is not exercised by auth-surface wiring tests")
        }
    }

    #[tokio::test]
    async fn env_auth_surface_deactivates_sso_local_trigger_access_without_sso() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let user_store_path = tmp.path().join("reborn-local-dev.db");
        let tenant_id = TenantId::new("env-auth-local-access-tenant").expect("tenant id");
        let agent_id = AgentId::new("env-auth-local-access-agent").expect("agent id");
        let stale_user_id = UserId::new("env-auth-local-access-stale").expect("user id");
        let access_store = open_local_trigger_access_store(&user_store_path)
            .await
            .expect("open local access store");
        access_store
            .seed_local_access(ironclaw_reborn_composition::LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &stale_user_id,
                agent_id: Some(&agent_id),
                project_id: None,
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
            })
            .await
            .expect("seed stale SSO access");

        let surface = build_webui_auth_surface(
            None,
            &user_store_path,
            tenant_id.clone(),
            SecretString::from("operator-session-secret".to_string()),
            Arc::new(OneToken),
            Some(LocalTriggerAccessBootstrapConfig {
                tenant_id: tenant_id.clone(),
                agent_id: agent_id.clone(),
                project_id: None,
            }),
        )
        .await
        .expect("build env auth surface");

        assert!(
            surface
                .authenticator
                .authenticate("env-token")
                .await
                .is_some(),
            "env auth remains active when no SSO providers are configured"
        );
        assert!(
            surface.public_mount.is_none(),
            "no SSO providers means no public login mount"
        );
        let denied = access_store
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id,
                creator_user_id: stale_user_id,
                agent_id: Some(agent_id),
                project_id: None,
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check stale access");
        assert_eq!(
            denied,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Denied {
                reason: "trigger creator does not have active local access for this scope"
                    .to_string(),
            }
        );
    }

    #[tokio::test]
    async fn sso_auth_surface_reconciles_existing_admitted_users_for_local_trigger_access() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let user_store_path = tmp.path().join("reborn-local-dev.db");
        let user_store = open_webui_user_store(&user_store_path)
            .await
            .expect("open user store");
        let access_store = open_local_trigger_access_store(&user_store_path)
            .await
            .expect("open local access store");
        let tenant_id = TenantId::new("sso-auth-surface-tenant").expect("tenant id");
        let agent_id = AgentId::new("sso-auth-surface-agent").expect("agent id");
        let project_id = ProjectId::new("sso-auth-surface-project").expect("project id");
        let admitted_user_id = user_store
            .resolve_or_create(ResolveIdentity {
                provider: "google",
                provider_user_id: "g-admitted",
                email: Some("alice@example.com"),
                email_verified: true,
                display_name: None,
            })
            .await
            .expect("create admitted user");
        let stale_user_id = UserId::new("sso-auth-surface-stale").expect("stale user id");
        access_store
            .seed_local_access(ironclaw_reborn_composition::LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &stale_user_id,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
            })
            .await
            .expect("seed stale access");

        let sso = SsoStartupConfig {
            providers: vec![Arc::new(StubProvider(
                OAuthProviderName::new("google").expect("provider name"),
            ))],
            base_url: "https://app.example".to_string(),
            allowed_email_domains: vec!["example.com".to_string()],
        };
        let _surface = build_webui_auth_surface(
            Some(sso),
            &user_store_path,
            tenant_id.clone(),
            SecretString::from("operator-session-secret".to_string()),
            Arc::new(OneToken),
            Some(LocalTriggerAccessBootstrapConfig {
                tenant_id: tenant_id.clone(),
                agent_id: agent_id.clone(),
                project_id: Some(project_id.clone()),
            }),
        )
        .await
        .expect("build auth surface");

        let allowed = access_store
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id: tenant_id.clone(),
                creator_user_id: admitted_user_id,
                agent_id: Some(agent_id.clone()),
                project_id: Some(project_id.clone()),
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check admitted access");
        assert_eq!(
            allowed,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Allowed
        );

        let denied = access_store
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id,
                creator_user_id: stale_user_id,
                agent_id: Some(agent_id),
                project_id: Some(project_id),
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check stale access");
        assert_eq!(
            denied,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Denied {
                reason: "trigger creator does not have active local access for this scope"
                    .to_string(),
            }
        );
    }

    #[tokio::test]
    async fn sso_auth_surface_reconciles_empty_admission_for_local_trigger_access() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let user_store_path = tmp.path().join("reborn-local-dev.db");
        let access_store = open_local_trigger_access_store(&user_store_path)
            .await
            .expect("open local access store");
        let tenant_id = TenantId::new("sso-auth-empty-tenant").expect("tenant id");
        let agent_id = AgentId::new("sso-auth-empty-agent").expect("agent id");
        let project_id = ProjectId::new("sso-auth-empty-project").expect("project id");
        let stale_user_id = UserId::new("sso-auth-empty-stale").expect("stale user id");
        access_store
            .seed_local_access(ironclaw_reborn_composition::LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &stale_user_id,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
            })
            .await
            .expect("seed stale access");

        let sso = SsoStartupConfig {
            providers: vec![Arc::new(StubProvider(
                OAuthProviderName::new("google").expect("provider name"),
            ))],
            base_url: "https://app.example".to_string(),
            allowed_email_domains: vec!["example.com".to_string()],
        };
        let _surface = build_webui_auth_surface(
            Some(sso),
            &user_store_path,
            tenant_id.clone(),
            SecretString::from("operator-session-secret".to_string()),
            Arc::new(OneToken),
            Some(LocalTriggerAccessBootstrapConfig {
                tenant_id: tenant_id.clone(),
                agent_id: agent_id.clone(),
                project_id: Some(project_id.clone()),
            }),
        )
        .await
        .expect("build auth surface");

        let denied = access_store
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id,
                creator_user_id: stale_user_id,
                agent_id: Some(agent_id),
                project_id: Some(project_id),
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check stale access");
        assert_eq!(
            denied,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Denied {
                reason: "trigger creator does not have active local access for this scope"
                    .to_string(),
            }
        );
    }
}
