//! Pluggable mapping from a normalized provider profile to a
//! [`UserId`].
//!
//! The composition layer in this crate intentionally does not own a
//! user database — the same crate is used by the standalone
//! `ironclaw-reborn` CLI (single operator, no user table) and by
//! production deployments (DB-backed user table). Host code supplies
//! whichever [`UserDirectory`] impl matches its deployment shape.
//!
//! The trait deliberately takes only the [`OAuthUserProfile`] —
//! providers do not pass raw token bodies through to the directory.
//! That keeps directory impls from accidentally depending on
//! provider-specific claim shapes.

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use thiserror::Error;

use super::profile::OAuthUserProfile;
use super::provider_name::OAuthProviderName;

/// Errors raised by a [`UserDirectory`] impl.
#[derive(Debug, Error)]
pub enum UserDirectoryError {
    /// The directory does not recognize the supplied profile and
    /// declines to create a new user. Surfaces a `403` from the
    /// OAuth callback.
    #[error("user is not recognized")]
    Unknown,
    /// Backend error (database unreachable, transaction conflict,
    /// etc.). Surfaces a `503` from the OAuth callback so operators
    /// can distinguish infrastructure faults from auth misses.
    #[error("user directory backend error: {0}")]
    Backend(String),
}

/// Trait host composition implements to map an OAuth provider's
/// normalized profile to a [`UserId`].
///
/// # Security
///
/// Production implementations must require
/// `profile.email_verified == true` before matching or linking an
/// account by email. Do not treat an unverified email claim as an
/// authoritative account identifier; fall back to a provider-unique
/// identifier or reject the login instead.
#[async_trait]
pub trait UserDirectory: Send + Sync + 'static {
    /// Resolve a `(provider, profile)` pair to the user id the
    /// session should be issued for. Impls may create a new user,
    /// link to an existing one by verified email, or reject the
    /// login entirely.
    async fn resolve(
        &self,
        provider: &OAuthProviderName,
        profile: &OAuthUserProfile,
    ) -> Result<UserId, UserDirectoryError>;
}

/// Local-dev / single-operator default impl: derive the
/// [`UserId`] from the verified provider email (lowercased) or fall
/// back to `{provider}:{provider_user_id}` when no verified email
/// is available. Production deployments should swap this for a
/// DB-backed impl that joins to the real user table.
///
/// Gated behind `dev-in-memory-session` for the same reason
/// [`crate::InMemorySessionStore`] is: a production binary cannot
/// accidentally name this impl as its `UserDirectory` — that would
/// trust the provider's `sub` blindly without a join against the
/// installation's user table.
#[cfg(any(test, feature = "dev-in-memory-session"))]
pub struct EmailUserDirectory;

#[cfg(any(test, feature = "dev-in-memory-session"))]
#[async_trait]
impl UserDirectory for EmailUserDirectory {
    async fn resolve(
        &self,
        provider: &OAuthProviderName,
        profile: &OAuthUserProfile,
    ) -> Result<UserId, UserDirectoryError> {
        let candidate = if profile.email_verified
            && let Some(email) = &profile.email
        {
            email.to_ascii_lowercase()
        } else {
            format!("{provider}:{}", profile.provider_user_id)
        };
        UserId::new(&candidate).map_err(|err| UserDirectoryError::Backend(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(email: Option<&str>, verified: bool, sub: &str) -> OAuthUserProfile {
        OAuthUserProfile {
            provider_user_id: sub.to_string(),
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

    fn google() -> OAuthProviderName {
        OAuthProviderName::new("google").unwrap()
    }

    #[tokio::test]
    async fn verified_email_becomes_user_id_lowercased() {
        let dir = EmailUserDirectory;
        let uid = dir
            .resolve(
                &google(),
                &profile(Some("Alice@Example.com"), true, "g-123"),
            )
            .await
            .expect("resolve");
        assert_eq!(uid.as_str(), "alice@example.com");
    }

    #[tokio::test]
    async fn unverified_email_falls_back_to_provider_sub() {
        let dir = EmailUserDirectory;
        let uid = dir
            .resolve(
                &google(),
                &profile(Some("alice@example.com"), false, "g-123"),
            )
            .await
            .expect("resolve");
        assert_eq!(uid.as_str(), "google:g-123");
    }
}
