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
use ironclaw_reborn_composition::host_api::UserId;
use ironclaw_reborn_composition::{RebornLibSqlUserStore, ResolveIdentity};
use ironclaw_reborn_webui_ingress::{
    OAuthProviderName, OAuthUserProfile, UserDirectory, UserDirectoryError,
};

/// Admission + persistence adapter implementing the ingress
/// [`UserDirectory`] seam.
pub(crate) struct WebuiUserDirectory {
    store: Arc<RebornLibSqlUserStore>,
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
            allowed_email_domains,
        }
    }

    /// Whether `profile` clears the fail-closed admission gate: a
    /// verified email whose domain is on the allowlist. An unverified
    /// email (untrustworthy domain), a missing email, or an off-list
    /// domain is denied.
    fn admits(&self, profile: &OAuthUserProfile) -> bool {
        if !profile.email_verified {
            return false;
        }
        let Some(email) = profile.email.as_deref() else {
            return false;
        };
        let Some(domain) = email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()) else {
            return false;
        };
        self.allowed_email_domains
            .iter()
            .any(|allowed| allowed == &domain)
    }
}

#[async_trait]
impl UserDirectory for WebuiUserDirectory {
    async fn resolve(
        &self,
        provider: &OAuthProviderName,
        profile: &OAuthUserProfile,
    ) -> Result<UserId, UserDirectoryError> {
        if !self.admits(profile) {
            // Fail closed: the callback maps `Unknown` to a 403 redirect
            // and mints no session.
            return Err(UserDirectoryError::Unknown);
        }
        self.store
            .resolve_or_create(ResolveIdentity {
                provider: provider.as_str(),
                provider_user_id: profile.provider_user_id.as_str(),
                email: profile.email.as_deref(),
                email_verified: profile.email_verified,
                display_name: profile.display_name.as_deref(),
            })
            .await
            .map_err(|err| UserDirectoryError::Backend(err.to_string()))
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
}
