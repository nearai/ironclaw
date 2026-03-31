//! OAuth/social login support for the web gateway.
//!
//! Provides direct OAuth authentication via Google and GitHub (with Apple,
//! email/phone, and NEAR wallet planned as follow-ups). On successful OAuth,
//! the system creates or links a user via the existing `UserStore`, issues an
//! API token, and sets it as an HttpOnly cookie.

pub mod providers;
pub mod state_store;

use serde::{Deserialize, Serialize};

/// User profile information extracted from an OAuth provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUserProfile {
    /// Provider-specific unique user identifier (Google `sub`, GitHub user ID).
    pub provider_user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Full JSON profile payload from the provider.
    pub raw: serde_json::Value,
}

/// OAuth-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("OAuth provider not configured: {0}")]
    NotConfigured(String),
    #[error("Invalid OAuth state (CSRF mismatch or expired)")]
    InvalidState,
    #[error("Code exchange failed: {0}")]
    CodeExchange(String),
    #[error("Failed to fetch user profile: {0}")]
    ProfileFetch(String),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Rate limited")]
    RateLimited,
}
