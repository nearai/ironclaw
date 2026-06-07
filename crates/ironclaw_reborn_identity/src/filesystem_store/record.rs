//! Persisted record shapes for the filesystem identity store.
//!
//! These are the on-disk JSON bodies behind each scoped path. They live in
//! their own module so the substrate's data layout is reviewable in one place,
//! separate from the resolve/link/create logic that reads and writes them.

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct StoredUser {
    pub(super) email: Option<String>,
    pub(super) display_name: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct StoredExternalIdentity {
    pub(super) user_id: String,
    pub(super) email: Option<String>,
    pub(super) email_verified: bool,
    pub(super) created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct StoredVerifiedEmailIndex {
    pub(super) user_id: String,
}
