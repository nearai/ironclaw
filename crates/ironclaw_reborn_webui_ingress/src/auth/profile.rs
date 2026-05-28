//! Normalized OAuth user profile returned by every provider impl.
//!
//! Providers (Google today, GitHub / NEAR later) decode their
//! provider-specific token shape and project it down to this common
//! struct. The downstream user resolver and route handler consume
//! only this struct — they never see raw provider claims, so a future
//! provider cannot leak new fields through to user-resolution code
//! without a deliberate change here.

use serde::{Deserialize, Serialize};

/// Provider-normalized user profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUserProfile {
    /// Stable per-provider identifier for this user (e.g. Google
    /// `sub` claim, GitHub numeric id). Required.
    pub provider_user_id: String,
    /// Email address claimed by the provider. May be missing for
    /// providers/scopes that do not include it.
    pub email: Option<String>,
    /// Whether the provider asserts the email is verified. Treat as
    /// `false` if missing — only trust verified emails when matching
    /// to existing identities.
    pub email_verified: bool,
    /// Optional display name from the provider.
    pub display_name: Option<String>,
}
