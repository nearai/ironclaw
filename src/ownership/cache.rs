//! In-process identity cache for the channel boundary.
//!
//! Maps `(channel, external_id)` → [`UserId`] (id + role).
//! Read-through: populated after a DB-backed identity lookup succeeds.
//! Explicit eviction happens on pairing removal and user invalidation paths.
//! No TTL — role changes are not implemented yet; add eviction at the write
//! site when they are.

use std::collections::HashMap;
use std::sync::RwLock;

use crate::ownership::UserId;

/// In-process cache: `(channel, external_id)` → [`UserId`].
///
/// All methods take `&self` — interior mutability via `RwLock`.
pub struct OwnershipCache {
    identities: RwLock<HashMap<(String, String), UserId>>,
}

impl OwnershipCache {
    pub fn new() -> Self {
        Self {
            identities: RwLock::new(HashMap::new()),
        }
    }

    /// Look up the [`UserId`] for `(channel, external_id)`. Returns `None` on miss.
    pub fn get(&self, channel: &str, external_id: &str) -> Option<UserId> {
        let key = (channel.to_string(), external_id.to_string());
        self.identities
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .cloned()
    }

    /// Insert or update an entry after a successful identity resolution.
    pub fn insert(&self, channel: &str, external_id: &str, identity: UserId) {
        let key = (channel.to_string(), external_id.to_string());
        self.identities
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, identity);
    }

    /// Remove an entry. Called on pairing removal or user deactivation.
    pub fn evict(&self, channel: &str, external_id: &str) {
        let key = (channel.to_string(), external_id.to_string());
        self.identities
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key);
    }

    /// Evict all cached entries belonging to a specific owner.
    /// Called when a user is deactivated or their role changes.
    pub fn evict_user(&self, owner_id: &str) {
        let mut map = self.identities.write().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, identity| identity.as_str() != owner_id);
    }
}

impl Default for OwnershipCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ownership::UserRole;

    fn alice() -> UserId {
        UserId::from_trusted("alice".into(), UserRole::Regular)
    }

    fn admin() -> UserId {
        UserId::from_trusted("admin".into(), UserRole::Admin)
    }

    #[test]
    fn test_cache_miss_returns_none() {
        let cache = OwnershipCache::new();
        assert!(cache.get("telegram", "123").is_none());
    }

    #[test]
    fn test_insert_then_get() {
        let cache = OwnershipCache::new();
        cache.insert("telegram", "123", alice());
        let result = cache.get("telegram", "123");
        assert!(result.is_some());
        assert_eq!(result.unwrap().as_str(), "alice");
    }

    #[test]
    fn test_evict_removes_entry() {
        let cache = OwnershipCache::new();
        cache.insert("telegram", "123", alice());
        cache.evict("telegram", "123");
        assert!(cache.get("telegram", "123").is_none());
    }

    #[test]
    fn test_different_channels_are_independent() {
        let cache = OwnershipCache::new();
        cache.insert("telegram", "123", alice());
        assert!(cache.get("discord", "123").is_none());
    }

    #[test]
    fn test_insert_overwrites_existing() {
        let cache = OwnershipCache::new();
        cache.insert("telegram", "123", alice());
        cache.insert("telegram", "123", admin());
        let result = cache.get("telegram", "123").unwrap();
        assert_eq!(result.as_str(), "admin");
    }

    #[test]
    fn test_evict_nonexistent_is_noop() {
        let cache = OwnershipCache::new();
        // Should not panic
        cache.evict("telegram", "nonexistent");
    }

    #[test]
    fn test_admin_role_preserved() {
        let cache = OwnershipCache::new();
        cache.insert("telegram", "123", admin());
        let result = cache.get("telegram", "123").unwrap();
        assert_eq!(result.role(), UserRole::Admin);
    }
}
