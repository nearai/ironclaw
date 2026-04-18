//! Lease manager — grants, validates, and expires capability leases.

use std::collections::HashMap;

use chrono::Utc;
use tokio::sync::RwLock;

use crate::types::capability::{CapabilityLease, GrantedActions, LeaseId};
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
    ///
    /// Returns `EngineError::Effect` if `duration` is non-positive or
    /// `max_uses` is zero — these would create immediately-expired or
    /// unusable leases.
    pub async fn grant(
        &self,
        thread_id: ThreadId,
        capability_name: impl Into<String>,
        granted_actions: GrantedActions,
        duration: Option<chrono::Duration>,
        max_uses: Option<u32>,
    ) -> Result<CapabilityLease, EngineError> {
        if let Some(d) = duration
            && d <= chrono::Duration::zero()
        {
            return Err(EngineError::Effect {
                reason: format!("lease duration must be positive, got {}s", d.num_seconds()),
            });
        }
        if let Some(0) = max_uses {
            return Err(EngineError::Effect {
                reason: "lease max_uses must be > 0".into(),
            });
        }

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
            revoked_reason: None,
        };
        self.active.write().await.insert(lease.id, lease.clone());
        Ok(lease)
    }

    /// Check whether a lease is still valid. Returns the lease if valid.
    pub async fn check(&self, lease_id: LeaseId) -> Result<CapabilityLease, EngineError> {
        let leases = self.active.read().await;
        let lease = leases
            .get(&lease_id)
            .ok_or_else(|| EngineError::LeaseNotFound {
                lease_id: format!("{lease_id:?}"),
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

    /// Refund one lease use after an execution was interrupted before the
    /// action completed.
    pub async fn refund_use(&self, lease_id: LeaseId) -> Result<(), EngineError> {
        let mut leases = self.active.write().await;
        let lease = leases
            .get_mut(&lease_id)
            .ok_or_else(|| EngineError::LeaseExpired {
                capability_name: format!("lease {lease_id:?} not found"),
            })?;
        lease.refund_use();
        Ok(())
    }

    /// Update the granted actions for an existing lease in place.
    pub async fn update_granted_actions(
        &self,
        lease_id: LeaseId,
        granted_actions: GrantedActions,
    ) -> Result<CapabilityLease, EngineError> {
        let mut leases = self.active.write().await;
        let lease = leases
            .get_mut(&lease_id)
            .ok_or_else(|| EngineError::LeaseNotFound {
                lease_id: format!("{lease_id:?}"),
            })?;
        lease.granted_actions = granted_actions;
        Ok(lease.clone())
    }

    /// Revoke a lease by ID with a reason for audit trail.
    pub async fn revoke(&self, lease_id: LeaseId, reason: &str) {
        let mut leases = self.active.write().await;
        if let Some(lease) = leases.get_mut(&lease_id) {
            lease.revoked = true;
            lease.revoked_reason = Some(reason.to_string());
            tracing::debug!(
                lease_id = ?lease_id,
                capability = %lease.capability_name,
                reason,
                "lease revoked"
            );
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
        let hyphenated = action_name.replace('_', "-");
        let underscored = action_name.replace('-', "_");
        let leases = self.active.read().await;
        leases
            .values()
            .find(|l| {
                l.thread_id == thread_id
                    && l.is_valid()
                    && (l.covers_action(action_name)
                        || l.covers_action(&hyphenated)
                        || l.covers_action(&underscored))
            })
            .cloned()
    }

    /// Derive child leases from a parent thread's active leases.
    ///
    /// Implements intersection semantics: the child gets only leases for
    /// actions that are both in the parent's active set AND in the
    /// `requested_actions` set. If `requested_actions` is `None`, the child
    /// inherits all of the parent's valid leases.
    ///
    /// Invariants:
    /// - A child can never have more privileges than its parent.
    /// - Child leases inherit the parent's expiry (never outlive parent).
    /// - Child leases inherit the parent's remaining budget.
    /// - Expired parent leases yield no child leases.
    ///
    /// **Budget note:** `max_uses` is a point-in-time snapshot of
    /// `parent.uses_remaining`. Parent and child budgets are independent after
    /// derivation — the parent can keep consuming its own lease. This means the
    /// combined usage (parent + child) may exceed the parent's original
    /// `max_uses`. This is intentional: the child gets an at-spawn budget
    /// rather than a shared atomic counter, which avoids cross-thread
    /// contention. Callers needing strict global budgets should use
    /// time-based expiry or cost-based limits at the thread level.
    pub async fn derive_child_leases(
        &self,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
        requested_actions: Option<&std::collections::HashSet<String>>,
    ) -> Vec<CapabilityLease> {
        let parent_leases = self.active_for_thread(parent_thread_id).await;
        let mut child_leases = Vec::new();

        for parent in &parent_leases {
            if !parent.is_valid() {
                continue;
            }

            let child_grants = match requested_actions {
                Some(req) => {
                    match &parent.granted_actions {
                        GrantedActions::All => {
                            // Parent is wildcard. Child gets only the
                            // requested subset, NOT a wildcard.
                            GrantedActions::Specific(req.iter().cloned().collect())
                        }
                        GrantedActions::Specific(parent_actions) => {
                            // Intersection: only actions in both parent and request.
                            let intersection: Vec<String> = parent_actions
                                .iter()
                                .filter(|a| req.contains(*a))
                                .cloned()
                                .collect();
                            GrantedActions::Specific(intersection)
                        }
                    }
                }
                None => parent.granted_actions.clone(),
            };

            // Skip if intersection is empty (no matching actions)
            if let GrantedActions::Specific(ref actions) = child_grants
                && actions.is_empty()
                && requested_actions.is_some()
            {
                continue;
            }

            child_leases.push(CapabilityLease {
                id: LeaseId::new(),
                thread_id: child_thread_id,
                capability_name: parent.capability_name.clone(),
                granted_actions: child_grants,
                granted_at: Utc::now(),
                expires_at: parent.expires_at,   // never outlive parent
                max_uses: parent.uses_remaining, // budget from parent's remaining
                uses_remaining: parent.uses_remaining,
                revoked: false,
                revoked_reason: None,
            });
        }

        // Batch insert under a single write lock (M2: avoid per-iteration locking)
        {
            let mut active = self.active.write().await;
            for child in &child_leases {
                active.insert(child.id, child.clone());
            }
        }

        child_leases
    }

    /// Atomically find the lease for an action and consume one use.
    ///
    /// Avoids the TOCTOU race between `find_lease_for_action` (read lock) and
    /// `consume_use` (write lock) — both happen under a single write lock.
    /// Returns the lease snapshot (post-consume) if found and valid.
    pub async fn find_and_consume(
        &self,
        thread_id: ThreadId,
        action_name: &str,
    ) -> Result<CapabilityLease, EngineError> {
        let mut leases = self.active.write().await;
        let lease = leases
            .values_mut()
            .find(|l| l.thread_id == thread_id && l.is_valid() && l.covers_action(action_name))
            .ok_or_else(|| EngineError::LeaseNotFound {
                lease_id: format!("no valid lease for action '{action_name}'"),
            })?;

        if !lease.consume_use() {
            return Err(EngineError::LeaseExpired {
                capability_name: lease.capability_name.clone(),
            });
        }

        Ok(lease.clone())
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
    use crate::types::capability::GrantedActions;
    use crate::types::thread::ThreadId;

    #[tokio::test]
    async fn grant_and_check() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr
            .grant(tid, "github", GrantedActions::All, None, None)
            .await
            .unwrap();
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
        let lease = mgr
            .grant(tid, "github", GrantedActions::All, None, Some(2))
            .await
            .unwrap();
        assert!(mgr.consume_use(lease.id).await.is_ok());
        assert!(mgr.consume_use(lease.id).await.is_ok());
        assert!(mgr.consume_use(lease.id).await.is_err());
    }

    #[tokio::test]
    async fn refund_use_restores_consumed_budget() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr
            .grant(tid, "github", GrantedActions::All, None, Some(2))
            .await
            .unwrap();
        mgr.consume_use(lease.id).await.unwrap();
        let consumed = mgr.check(lease.id).await.unwrap();
        assert_eq!(consumed.uses_remaining, Some(1));
        mgr.refund_use(lease.id).await.unwrap();
        let restored = mgr.check(lease.id).await.unwrap();
        assert_eq!(restored.uses_remaining, Some(2));
    }

    #[tokio::test]
    async fn revoke_invalidates() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr
            .grant(tid, "github", GrantedActions::All, None, None)
            .await
            .unwrap();
        mgr.revoke(lease.id, "test").await;
        assert!(mgr.check(lease.id).await.is_err());
    }

    #[tokio::test]
    async fn expire_stale_removes_revoked() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let lease = mgr
            .grant(tid, "github", GrantedActions::All, None, None)
            .await
            .unwrap();
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
        mgr.grant(t1, "github", GrantedActions::All, None, None)
            .await
            .unwrap();
        mgr.grant(t1, "memory", GrantedActions::All, None, None)
            .await
            .unwrap();
        mgr.grant(t2, "slack", GrantedActions::All, None, None)
            .await
            .unwrap();
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
            GrantedActions::Specific(vec!["create_issue".into(), "list_prs".into()]),
            None,
            None,
        )
        .await
        .unwrap();
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
    async fn negative_duration_rejected() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let result = mgr
            .grant(
                tid,
                "github",
                GrantedActions::All,
                Some(chrono::Duration::seconds(-10)),
                None,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn zero_max_uses_rejected() {
        let mgr = LeaseManager::new();
        let tid = ThreadId::new();
        let result = mgr
            .grant(tid, "github", GrantedActions::All, None, Some(0))
            .await;
        assert!(result.is_err());
    }

    // ── derive_child_leases ──────────────────────────────────

    #[tokio::test]
    async fn test_child_inherits_subset_of_parent() {
        let mgr = LeaseManager::new();
        let parent = ThreadId::new();
        let child = ThreadId::new();

        mgr.grant(
            parent,
            "tools",
            GrantedActions::Specific(vec!["A".into(), "B".into(), "C".into()]),
            None,
            None,
        )
        .await
        .unwrap();

        let mut requested = std::collections::HashSet::new();
        requested.insert("B".into());
        requested.insert("C".into());
        requested.insert("D".into()); // not in parent

        let child_leases = mgr
            .derive_child_leases(parent, child, Some(&requested))
            .await;
        assert_eq!(child_leases.len(), 1);
        assert!(child_leases[0].granted_actions.covers("B"));
        assert!(child_leases[0].granted_actions.covers("C"));
        assert!(!child_leases[0].granted_actions.covers("D"));
    }

    #[tokio::test]
    async fn test_child_never_exceeds_parent_expiry() {
        let mgr = LeaseManager::new();
        let parent = ThreadId::new();
        let child = ThreadId::new();

        let parent_lease = mgr
            .grant(
                parent,
                "tools",
                GrantedActions::Specific(vec!["read".into()]),
                Some(chrono::Duration::hours(1)),
                None,
            )
            .await
            .unwrap();

        let child_leases = mgr.derive_child_leases(parent, child, None).await;
        assert_eq!(child_leases.len(), 1);
        assert_eq!(child_leases[0].expires_at, parent_lease.expires_at);
    }

    #[tokio::test]
    async fn test_expired_parent_yields_empty_child() {
        let mgr = LeaseManager::new();
        let parent = ThreadId::new();
        let child = ThreadId::new();

        // Manually insert an already-expired lease (bypassing grant validation)
        let now = Utc::now();
        let expired_lease = CapabilityLease {
            id: LeaseId::new(),
            thread_id: parent,
            capability_name: "tools".into(),
            granted_actions: GrantedActions::Specific(vec!["read".into()]),
            granted_at: now,
            expires_at: Some(now - chrono::Duration::seconds(10)),
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        };
        mgr.active
            .write()
            .await
            .insert(expired_lease.id, expired_lease);

        let child_leases = mgr.derive_child_leases(parent, child, None).await;
        assert!(child_leases.is_empty());
    }

    #[tokio::test]
    async fn test_child_inherits_remaining_budget() {
        let mgr = LeaseManager::new();
        let parent = ThreadId::new();
        let child = ThreadId::new();

        let parent_lease = mgr
            .grant(
                parent,
                "tools",
                GrantedActions::Specific(vec!["read".into()]),
                None,
                Some(10),
            )
            .await
            .unwrap();

        // Consume 3 uses from parent
        mgr.consume_use(parent_lease.id).await.unwrap();
        mgr.consume_use(parent_lease.id).await.unwrap();
        mgr.consume_use(parent_lease.id).await.unwrap();

        let child_leases = mgr.derive_child_leases(parent, child, None).await;
        assert_eq!(child_leases.len(), 1);
        // Parent had 10, consumed 3, so 7 remaining
        assert_eq!(child_leases[0].uses_remaining, Some(7));
    }

    #[tokio::test]
    async fn test_child_with_none_inherits_all() {
        let mgr = LeaseManager::new();
        let parent = ThreadId::new();
        let child = ThreadId::new();

        mgr.grant(
            parent,
            "tools",
            GrantedActions::Specific(vec!["read".into(), "write".into()]),
            None,
            None,
        )
        .await
        .unwrap();

        let child_leases = mgr.derive_child_leases(parent, child, None).await;
        assert_eq!(child_leases.len(), 1);
        assert_eq!(child_leases[0].granted_actions.actions().len(), 2);
    }
}
