//! Host [`UserDirectory`] for the WebChat v2 SSO login surface.
//!
//! Thin adapter over the reborn-owned
//! [`RebornLibSqlUserStore`](ironclaw_reborn_composition::RebornLibSqlUserStore)
//! (reached through the composition facade, not a direct `ironclaw_reborn`
//! dependency): it applies the operator's email-domain admission policy
//! (fail-closed),
//! then delegates identity resolution/persistence to the store. Keeping
//! admission here — in the host adapter — leaves the storage layer pure
//! and the ingress trait seam unchanged, and keeps the durable schema in
//! `ironclaw_reborn` rather than in this command crate.
//!
//! Admission is the control that stops a configured provider from
//! becoming open registration: GitHub has no org/team allowlist and
//! Google only an optional hosted-domain check, so without an explicit
//! verified-email-domain allowlist *any* Google/GitHub account could mint
//! a session on a protected WebUI. `serve` refuses to start when SSO
//! providers are configured without an allowlist, so the list is never
//! empty in production.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_reborn_composition::host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_reborn_composition::{
    LocalTriggerAccessRole, LocalTriggerAccessSeed, LocalTriggerAccessSource,
    RebornLibSqlLocalTriggerAccessStore, RebornLibSqlUserStore, ResolveIdentity,
};
use ironclaw_reborn_webui_ingress::{
    OAuthProviderName, OAuthUserProfile, UserDirectory, UserDirectoryError,
};

/// Admission + persistence adapter implementing the ingress
/// [`UserDirectory`] seam.
pub(crate) struct WebuiUserDirectory {
    store: Arc<RebornLibSqlUserStore>,
    local_trigger_access: Option<LocalTriggerAccessBootstrap>,
    /// Lowercased verified-email domains allowed to log in. Never empty
    /// in production — an empty list rejects every login (fail closed).
    allowed_email_domains: Vec<String>,
}

impl WebuiUserDirectory {
    pub(crate) fn new(
        store: Arc<RebornLibSqlUserStore>,
        allowed_email_domains: Vec<String>,
    ) -> Self {
        Self {
            store,
            local_trigger_access: None,
            allowed_email_domains,
        }
    }

    pub(crate) fn with_local_trigger_access(
        mut self,
        local_trigger_access: LocalTriggerAccessBootstrap,
    ) -> Self {
        self.local_trigger_access = Some(local_trigger_access);
        self
    }

    /// The verified email this profile is admitted on, if any: the first
    /// verified address whose domain is on the allowlist. Candidates are
    /// the canonical [`email`](OAuthUserProfile::email) (only when
    /// `email_verified`) followed by every entry in
    /// [`verified_emails`](OAuthUserProfile::verified_emails) — so a user
    /// whose primary address is off-list is still admitted on a verified
    /// secondary that is on it (GitHub returns the full set). Returns
    /// `None` (fail closed) when no verified candidate matches: an
    /// unverified-only profile, a missing email, or an off-list domain.
    ///
    /// The returned address is the one the user is linked/persisted under,
    /// so cross-provider account linking keys on the allowlisted email.
    fn admitted_email(&self, profile: &OAuthUserProfile) -> Option<String> {
        let canonical = profile
            .email
            .as_deref()
            .filter(|_| profile.email_verified)
            .into_iter();
        canonical
            .chain(profile.verified_emails.iter().map(String::as_str))
            .find(|email| self.domain_allowed(email))
            .map(str::to_string)
    }

    /// Whether `email`'s domain is on the operator allowlist
    /// (case-insensitive).
    fn domain_allowed(&self, email: &str) -> bool {
        email
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_ascii_lowercase())
            .is_some_and(|domain| self.allowed_email_domains.iter().any(|a| a == &domain))
    }
}

/// Local-dev trigger access seed configuration for users admitted through SSO.
pub(crate) struct LocalTriggerAccessBootstrap {
    store: Arc<RebornLibSqlLocalTriggerAccessStore>,
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
}

impl LocalTriggerAccessBootstrap {
    pub(crate) fn new(
        store: Arc<RebornLibSqlLocalTriggerAccessStore>,
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            store,
            tenant_id,
            agent_id,
            project_id,
        }
    }

    async fn seed_for_user(&self, user_id: &UserId) -> Result<(), UserDirectoryError> {
        self.store
            .seed_local_access(LocalTriggerAccessSeed {
                tenant_id: &self.tenant_id,
                user_id,
                agent_id: Some(&self.agent_id),
                project_id: self.project_id.as_ref(),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
            })
            .await
            .map_err(|err| UserDirectoryError::Backend(err.to_string()))
    }
}

#[async_trait]
impl UserDirectory for WebuiUserDirectory {
    async fn resolve(
        &self,
        provider: &OAuthProviderName,
        profile: &OAuthUserProfile,
    ) -> Result<UserId, UserDirectoryError> {
        // Fail closed: an unadmitted profile maps to a 403 redirect and
        // mints no session. The admitted address is what we link/persist
        // on, so an allowlisted verified secondary email wins over an
        // off-list primary.
        let Some(admitted_email) = self.admitted_email(profile) else {
            // Redacted diagnostic so an operator can see which domain to add
            // to IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS. Logs only the
            // email DOMAINS the provider returned (never the local-part or
            // full address) plus whether the canonical email was verified.
            let candidate_domains: std::collections::BTreeSet<String> = profile
                .email
                .as_deref()
                .into_iter()
                .chain(profile.verified_emails.iter().map(String::as_str))
                .filter_map(|email| email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()))
                .collect();
            tracing::warn!(
                target: "ironclaw::reborn::webui_ingress::auth",
                provider = provider.as_str(),
                email_verified = profile.email_verified,
                candidate_domains = ?candidate_domains,
                allowed_domains = ?self.allowed_email_domains,
                "WebChat SSO admission denied: no verified email on an allowlisted domain"
            );
            return Err(UserDirectoryError::Unknown);
        };
        let user_id = self
            .store
            .resolve_or_create(ResolveIdentity {
                provider: provider.as_str(),
                provider_user_id: profile.provider_user_id.as_str(),
                email: Some(admitted_email.as_str()),
                email_verified: true,
                display_name: profile.display_name.as_deref(),
            })
            .await
            .map_err(|err| UserDirectoryError::Backend(err.to_string()))?;
        if let Some(local_trigger_access) = &self.local_trigger_access {
            local_trigger_access.seed_for_user(&user_id).await?;
        }
        Ok(user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn directory(domains: &[&str]) -> WebuiUserDirectory {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Leak the tempdir so the libSQL file outlives the test body.
        let path = tmp.keep().join("reborn-local-dev.db");
        // Open through the same composition facade production uses, so the
        // CLI test needs no direct libSQL dependency.
        let store = ironclaw_reborn_composition::open_webui_user_store(&path)
            .await
            .expect("store");
        WebuiUserDirectory::new(store, domains.iter().map(|d| d.to_string()).collect())
    }

    fn google() -> OAuthProviderName {
        OAuthProviderName::new("google").expect("provider")
    }

    fn profile(email: Option<&str>, verified: bool) -> OAuthUserProfile {
        OAuthUserProfile {
            provider_user_id: "g-1".to_string(),
            email: email.map(str::to_string),
            email_verified: verified,
            verified_emails: email
                .filter(|_| verified)
                .map(str::to_string)
                .into_iter()
                .collect(),
            display_name: None,
        }
    }

    #[tokio::test]
    async fn verified_allowed_domain_is_admitted() {
        let dir = directory(&["example.com"]).await;
        let user = dir
            .resolve(&google(), &profile(Some("alice@example.com"), true))
            .await
            .expect("an allowed verified domain must be admitted");
        assert!(!user.as_str().is_empty());
    }

    #[tokio::test]
    async fn disallowed_domain_is_rejected_without_minting() {
        let dir = directory(&["example.com"]).await;
        let err = dir
            .resolve(&google(), &profile(Some("mallory@evil.test"), true))
            .await
            .expect_err("an off-allowlist domain must be rejected");
        assert!(matches!(err, UserDirectoryError::Unknown));
    }

    #[tokio::test]
    async fn unverified_email_in_allowed_domain_is_rejected() {
        let dir = directory(&["example.com"]).await;
        let err = dir
            .resolve(&google(), &profile(Some("alice@example.com"), false))
            .await
            .expect_err("an unverified email must be rejected even on an allowed domain");
        assert!(matches!(err, UserDirectoryError::Unknown));
    }

    #[tokio::test]
    async fn missing_email_is_rejected() {
        let dir = directory(&["example.com"]).await;
        let err = dir
            .resolve(&google(), &profile(None, true))
            .await
            .expect_err("a profile without an email cannot clear a domain allowlist");
        assert!(matches!(err, UserDirectoryError::Unknown));
    }

    #[tokio::test]
    async fn domain_match_is_case_insensitive() {
        let dir = directory(&["example.com"]).await;
        dir.resolve(&google(), &profile(Some("Alice@Example.COM"), true))
            .await
            .expect("domain comparison must be case-insensitive");
    }

    #[tokio::test]
    async fn allowlisted_verified_secondary_email_is_admitted_over_offlist_primary() {
        // GitHub-shaped profile: the primary verified email is off-list,
        // but a verified secondary address is on the allowlist. The user
        // must be admitted (and linked) on the allowlisted secondary, not
        // denied for the primary. Regression for admission only checking
        // the single canonical `email`.
        let dir = directory(&["company.com"]).await;
        let profile = OAuthUserProfile {
            provider_user_id: "gh-42".to_string(),
            email: Some("alice@gmail.com".to_string()),
            email_verified: true,
            verified_emails: vec![
                "alice@gmail.com".to_string(),
                "alice@company.com".to_string(),
            ],
            display_name: None,
        };
        let user = dir
            .resolve(&google(), &profile)
            .await
            .expect("a verified secondary email on the allowlist must be admitted");
        assert!(!user.as_str().is_empty());
    }

    #[tokio::test]
    async fn no_verified_email_on_allowlist_is_rejected_despite_other_verified() {
        // All verified addresses are off-list → fail closed, even though
        // the account has verified emails.
        let dir = directory(&["company.com"]).await;
        let profile = OAuthUserProfile {
            provider_user_id: "gh-43".to_string(),
            email: Some("bob@gmail.com".to_string()),
            email_verified: true,
            verified_emails: vec!["bob@gmail.com".to_string(), "bob@outlook.com".to_string()],
            display_name: None,
        };
        let err = dir
            .resolve(&google(), &profile)
            .await
            .expect_err("no verified email on the allowlist must be rejected");
        assert!(matches!(err, UserDirectoryError::Unknown));
    }

    #[tokio::test]
    async fn sso_user_directory_seeds_local_trigger_access_for_admitted_user() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.keep().join("reborn-local-dev.db");
        let user_store = ironclaw_reborn_composition::open_webui_user_store(&path)
            .await
            .expect("open user store");
        let access_store = ironclaw_reborn_composition::open_local_trigger_access_store(&path)
            .await
            .expect("open access store");
        let tenant_id = TenantId::new("sso-access-tenant").expect("tenant id");
        let agent_id = AgentId::new("sso-access-agent").expect("agent id");
        let project_id = ProjectId::new("sso-access-project").expect("project id");
        let dir = WebuiUserDirectory::new(user_store, vec!["example.com".to_string()])
            .with_local_trigger_access(LocalTriggerAccessBootstrap::new(
                access_store.clone(),
                tenant_id.clone(),
                agent_id.clone(),
                Some(project_id.clone()),
            ));

        let user_id = dir
            .resolve(&google(), &profile(Some("alice@example.com"), true))
            .await
            .expect("admitted SSO profile resolves");

        assert!(
            access_store
                .has_active_local_access(&tenant_id, &user_id, Some(&agent_id), Some(&project_id))
                .await
                .expect("check local access"),
            "admitted SSO users get an exact local-dev trigger access row"
        );
    }

    #[tokio::test]
    async fn sso_user_directory_does_not_seed_local_trigger_access_for_unadmitted_user() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.keep().join("reborn-local-dev.db");
        let user_store = ironclaw_reborn_composition::open_webui_user_store(&path)
            .await
            .expect("open user store");
        let access_store = ironclaw_reborn_composition::open_local_trigger_access_store(&path)
            .await
            .expect("open access store");
        let tenant_id = TenantId::new("sso-access-reject-tenant").expect("tenant id");
        let agent_id = AgentId::new("sso-access-reject-agent").expect("agent id");
        let project_id = ProjectId::new("sso-access-reject-project").expect("project id");
        let dir = WebuiUserDirectory::new(user_store.clone(), vec!["example.com".to_string()])
            .with_local_trigger_access(LocalTriggerAccessBootstrap::new(
                access_store.clone(),
                tenant_id.clone(),
                agent_id.clone(),
                Some(project_id.clone()),
            ));

        let err = dir
            .resolve(&google(), &profile(Some("mallory@evil.test"), true))
            .await
            .expect_err("off-allowlist SSO profile must be rejected");
        assert!(matches!(err, UserDirectoryError::Unknown));

        let rejected_users = user_store
            .list_active_users_by_allowed_email_domains(&["evil.test".to_string()])
            .await
            .expect("list rejected-domain users");
        assert!(
            rejected_users.is_empty(),
            "rejected SSO profiles must fail before user resolution"
        );
        let sentinel_user_id = UserId::new("sso-access-reject-user").expect("user id");
        assert!(
            !access_store
                .has_active_local_access(
                    &tenant_id,
                    &sentinel_user_id,
                    Some(&agent_id),
                    Some(&project_id)
                )
                .await
                .expect("check local access"),
            "rejected SSO profiles must not seed local trigger access"
        );
    }
}
