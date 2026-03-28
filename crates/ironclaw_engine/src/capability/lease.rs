//! Lease manager — grants, validates, and expires capability leases.

use std::collections::HashMap;

use chrono::Utc;
use tokio::sync::RwLock;

use crate::types::capability::{CapabilityLease, LeaseId};
use crate::types::error::EngineError;
use crate::types::thread::ThreadId;

/// Manages the lifecycle of capability leases.
///
/// Leases are the mechanism by which threads gain access to capabilities.
/// They are scoped (time-limited, use-limited, action-restricted) to bound
/// the blast radius of any single thread.
pub struct LeaseManager {
    active: RwLock<HashMap<LeaseId, CapabilityLease>>,
}

impl LeaseManager {
    pub fn new() -> Self {
        Self {
            active: RwLock::new(HashMap::new()),
        }
    }

    /// Grant a new lease to a thread.
    pub async fn grant(
        &self,
        thread_id: ThreadId,
        capability_name: impl Into<String>,
        granted_actions: Vec<String>,
        duration: Option<chrono::Duration>,
        max_uses: Option<u32>,
    ) -> CapabilityLease {
        let now = Utc::now();
        let lease = CapabilityLease {
            id: LeaseId::new(),
            thread_id,
            capability_name: capability_name.into(),
            granted_actions,
            granted_at: now,
            expires_at: duration.map(|d| now + d),
            max_uses,
            uses_remaining: max_uses,
            revoked: false,
        };
        self.active.write().await.insert(lease.id, lease.clone());
        lease
    }

    /// Check whether a lease is still valid. Returns the lease if valid.
    pub async fn check(&self, lease_id: LeaseId) -> Result<CapabilityLease, EngineError> {
        let leases = self.active.read().await;
        let lease = leases
            .get(&lease_id)
            .ok_or_else(|| EngineError::LeaseExpired {
                capability_name: format!("lease {lease_id:?} not found"),
            })?;
        if !lease.is_valid() {
            return Err(EngineError::LeaseExpired {
                capability_name: lease.capability_name.clone(),
            });
        }
        Ok(lease.clone())
    }

    /// Consume one use of a lease. Returns error if the lease is invalid or exhausted.
    pub async fn consume_use(&self, lease_id: LeaseId) -> Result<(), EngineError> {
        let mut leases = self.active.write().await;
        let lease = leases
            .get_mut(&lease_id)
            .ok_or_else(|| EngineError::LeaseExpired {
                capability_name: format!("lease {lease_id:?} not found"),
            })?;
        if !lease.is_valid() {
            return Err(EngineError::LeaseExpired {
                capability_name: lease.capability_name.clone(),
            });
        }
        if !lease.consume_use() {
            return Err(EngineError::LeaseExpired {
                capability_name: lease.capability_name.clone(),
            });
        }
        Ok(())
    }

    /// Revoke a lease by ID.
    pub async fn revoke(&self, lease_id: LeaseId, _reason: &str) {
        let mut leases = self.active.write().await;
        if let Some(lease) = leases.get_mut(&lease_id) {
            lease.revoked = true;
        }
    }

    /// Remove all expired or revoked leases from the active set.
    pub async fn expire_stale(&self) -> usize {
        let mut leases = self.active.write().await;
        let before = leases.len();
        leases.retain(|_, lease| lease.is_valid());
        before - leases.len()
    }

    /// Get all active (valid) leases for a thread.
    pub async fn active_for_thread(&self, thread_id: ThreadId) -> Vec<CapabilityLease> {
        let leases = self.active.read().await;
        leases
            .values()
            .filter(|l| l.thread_id == thread_id && l.is_valid())
            .cloned()
            .collect()
    }

    /// Find the lease that grants a specific action to a thread.
    pub async fn find_lease_for_action(
        &self,
        thread_id: ThreadId,
        action_name: &str,
    ) -> Option<CapabilityLease> {
        let leases = self.active.read().await;
        leases
            .values()
            .find(|l| l.thread_id == thread_id && l.is_valid() && l.covers_action(action_name))
            .cloned()
    }
}

impl Default for LeaseManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::thread::ThreadId;

    #[tokio::test]
    async fn grant_and_check() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr.grant(tid, "github", vec![], None, None).await;
        assert!(mgr.check(lease.id).await.is_ok());
    }

    #[tokio::test]
    async fn check_nonexistent_fails() {
        let mgr = LeaseManager::new();
        assert!(mgr.check(LeaseId::new()).await.is_err());
    }

    #[tokio::test]
    async fn consume_use_works() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr.grant(tid, "github", vec![], None, Some(2)).await;
        assert!(mgr.consume_use(lease.id).await.is_ok());
        assert!(mgr.consume_use(lease.id).await.is_ok());
        assert!(mgr.consume_use(lease.id).await.is_err());
    }

    #[tokio::test]
    async fn revoke_invalidates() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr.grant(tid, "github", vec![], None, None).await;
        mgr.revoke(lease.id, "test").await;
        assert!(mgr.check(lease.id).await.is_err());
    }

    #[tokio::test]
    async fn expire_stale_removes_revoked() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr.grant(tid, "github", vec![], None, None).await;
        mgr.revoke(lease.id, "done").await;
        let removed = mgr.expire_stale().await;
        assert_eq!(removed, 1);
        assert!(mgr.active_for_thread(tid).await.is_empty());
    }

    #[tokio::test]
    async fn active_for_thread_filters_correctly() {
        let mgr = LeaseManager::new();
        let t1 = ThreadId::new();
        let t2 = ThreadId::new();
        mgr.grant(t1, "github", vec![], None, None).await;
        mgr.grant(t1, "memory", vec![], None, None).await;
        mgr.grant(t2, "slack", vec![], None, None).await;
        assert_eq!(mgr.active_for_thread(t1).await.len(), 2);
        assert_eq!(mgr.active_for_thread(t2).await.len(), 1);
    }

    #[tokio::test]
    async fn find_lease_for_action_respects_grants() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        mgr.grant(
            tid,
            "github",
            vec!["create_issue".into(), "list_prs".into()],
            None,
            None,
        )
        .await;
        assert!(
            mgr.find_lease_for_action(tid, "create_issue")
                .await
                .is_some()
        );
        assert!(
            mgr.find_lease_for_action(tid, "delete_repo")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn expired_lease_not_active() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr
            .grant(
                tid,
                "github",
                vec![],
                Some(chrono::Duration::seconds(-10)),
                None,
            )
            .await;
        assert!(mgr.check(lease.id).await.is_err());
        assert!(mgr.active_for_thread(tid).await.is_empty());
    }
}
