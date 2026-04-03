//! Core type definitions for the engine.
//!
//! All data structures live here. No async, no I/O — just types and
//! validation logic.

use std::borrow::Cow;

pub mod capability;
pub mod conversation;
pub mod error;
pub mod event;
pub mod memory;
pub mod message;
pub mod mission;
pub mod project;
pub mod provenance;
pub mod step;
pub mod thread;

pub const LEGACY_SHARED_OWNER_ID: &str = "system";
pub const SHARED_OWNER_ID: &str = "__shared__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerId<'a> {
    Shared,
    User(Cow<'a, str>),
}

/// Default user_id for backwards-compatible deserialization of records
/// created before multi-tenant isolation was added.
pub(crate) fn default_user_id() -> String {
    "legacy".to_string()
}

pub fn shared_owner_id() -> &'static str {
    SHARED_OWNER_ID
}

pub fn is_shared_owner(user_id: &str) -> bool {
    user_id == SHARED_OWNER_ID || user_id == LEGACY_SHARED_OWNER_ID
}

impl<'a> OwnerId<'a> {
    pub fn from_user_id(user_id: &'a str) -> Self {
        if is_shared_owner(user_id) {
            Self::Shared
        } else {
            Self::User(Cow::Borrowed(user_id))
        }
    }

    pub fn is_shared(&self) -> bool {
        matches!(self, Self::Shared)
    }

    pub fn matches_user(&self, user_id: &str) -> bool {
        matches!(self, Self::User(owner) if owner == user_id)
    }

    pub fn as_user_id(&self) -> &str {
        match self {
            Self::Shared => shared_owner_id(),
            Self::User(user_id) => user_id.as_ref(),
        }
    }
}

pub fn shared_owner_candidates() -> [&'static str; 2] {
    [SHARED_OWNER_ID, LEGACY_SHARED_OWNER_ID]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_id_maps_shared_and_user_values() {
        assert!(OwnerId::from_user_id(SHARED_OWNER_ID).is_shared());
        assert!(OwnerId::from_user_id(LEGACY_SHARED_OWNER_ID).is_shared());
        assert!(OwnerId::from_user_id("alice").matches_user("alice"));
    }
}
