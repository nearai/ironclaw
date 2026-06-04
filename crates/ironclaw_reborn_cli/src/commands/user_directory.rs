//! Host [`UserDirectory`] for the WebChat v2 SSO login surface.
//!
//! Thin adapter over the canonical Reborn identity resolver
//! ([`RebornIdentityResolver`](ironclaw_reborn_composition::RebornIdentityResolver),
//! reached through the composition facade): it applies the operator's
//! email-domain admission policy (fail-closed), then delegates identity
//! resolution/persistence to the canonical resolver as an `oauth`-surface
//! external identity. Keeping admission here — in the host adapter —
//! leaves the canonical resolver pure and the ingress trait seam
//! unchanged.
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
use ironclaw_reborn_composition::host_api::{TenantId, UserId};
use ironclaw_reborn_composition::{RebornIdentityResolver, ResolveExternalIdentity, SurfaceKind};
use ironclaw_reborn_webui_ingress::{
    OAuthProviderName, OAuthUserProfile, UserDirectory, UserDirectoryError,
};

/// Admission + persistence adapter implementing the ingress
/// [`UserDirectory`] seam.
pub(crate) struct WebuiUserDirectory {
    resolver: Arc<dyn RebornIdentityResolver>,
    /// Trusted host tenant the resolved OAuth identities are scoped to.
    /// Identity resolution and email-linking happen within this tenant.
    tenant_id: TenantId,
    /// Lowercased verified-email domains allowed to log in. Never empty
    /// in production — an empty list rejects every login (fail closed).
    allowed_email_domains: Vec<String>,
}

impl WebuiUserDirectory {
    pub(crate) fn new(
        resolver: Arc<dyn RebornIdentityResolver>,
        tenant_id: TenantId,
        allowed_email_domains: Vec<String>,
    ) -> Self {
        Self {
            resolver,
            tenant_id,
            allowed_email_domains,
        }
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
        // An OAuth login is an `oauth`-surface external identity: no adapter
        // installation, keyed by provider + subject within the host tenant.
        // The admitted (verified, allowlisted) email is what cross-provider
        // linking keys on, so it is the email handed to the resolver.
        self.resolver
            .resolve_or_create(ResolveExternalIdentity {
                tenant_id: &self.tenant_id,
                surface_kind: SurfaceKind::Oauth,
                provider_kind: provider.as_str(),
                provider_instance_id: None,
                external_subject_id: profile.provider_user_id.as_str(),
                email: Some(admitted_email.as_str()),
                email_verified: true,
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
        let resolver = ironclaw_reborn_composition::open_reborn_identity_resolver(&path)
            .await
            .expect("resolver");
        WebuiUserDirectory::new(
            resolver,
            TenantId::new("tenant-test").expect("tenant"),
            domains.iter().map(|d| d.to_string()).collect(),
        )
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
}
