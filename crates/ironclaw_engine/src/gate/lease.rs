//! Lease gate — denies tool calls with no valid capability lease.
//!
//! Priority 10: runs before all other gates (deny early if no lease).
//! This replaces v1's `ApprovalContext::is_blocked_or_default()`,
//! `check_approval_in_context()`, and the ad-hoc `allowed_tools: HashSet`
//! in lightweight routines.

use std::sync::Arc;

use async_trait::async_trait;

use crate::capability::lease::LeaseManager;
use crate::gate::{ExecutionGate, GateContext, GateDecision};

/// Gate that denies tool calls not covered by a valid capability lease.
///
/// Fail-closed: if no lease exists for the action, execution is denied.
/// This is the primary authorization gate for the engine.
pub struct LeaseGate {
    lease_manager: Arc<LeaseManager>,
    /// When true, skip lease checks (for interactive threads where
    /// leases are still being granted by the planner).
    permissive: bool,
}

impl LeaseGate {
    /// Create a lease gate that enforces lease checks.
    pub fn new(lease_manager: Arc<LeaseManager>) -> Self {
        Self {
            lease_manager,
            permissive: false,
        }
    }

    /// Create a permissive gate that allows all actions (for Foreground
    /// threads where interactive approval handles authorization).
    pub fn permissive(lease_manager: Arc<LeaseManager>) -> Self {
        Self {
            lease_manager,
            permissive: true,
        }
    }
}

#[async_trait]
impl ExecutionGate for LeaseGate {
    fn name(&self) -> &str {
        "lease"
    }

    fn priority(&self) -> u32 {
        10
    }

    async fn evaluate(&self, ctx: &GateContext<'_>) -> GateDecision {
        if self.permissive {
            return GateDecision::Allow;
        }

        match self
            .lease_manager
            .find_lease_for_action(ctx.thread_id, ctx.action_name)
            .await
        {
            Some(lease) if lease.is_valid() => GateDecision::Allow,
            _ => GateDecision::Deny {
                reason: format!(
                    "No valid lease for action '{}' on thread {}",
                    ctx.action_name, ctx.thread_id
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::ExecutionMode;
    use crate::types::capability::{ActionDef, EffectType, GrantedActions};
    use crate::types::thread::ThreadId;
    use std::collections::HashSet;

    fn action_def(name: &str) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: String::new(),
            parameters_schema: serde_json::json!({}),
            effects: vec![EffectType::ReadLocal],
            requires_approval: false,
        }
    }

    fn ctx<'a>(
        thread_id: ThreadId,
        action_def: &'a ActionDef,
        auto: &'a HashSet<String>,
        params: &'a serde_json::Value,
    ) -> GateContext<'a> {
        GateContext {
            user_id: "user1",
            thread_id,
            source_channel: "web",
            action_name: &action_def.name,
            call_id: "call_1",
            parameters: params,
            action_def,
            execution_mode: ExecutionMode::Autonomous,
            auto_approved: auto,
        }
    }

    #[tokio::test]
    async fn test_valid_lease_allows() {
        let mgr = Arc::new(LeaseManager::new());
        let tid = ThreadId::new();
        mgr.grant(
            tid,
            "tools",
            GrantedActions::Specific(vec!["read_file".into()]),
            None,
            None,
        )
        .await
        .unwrap();

        let gate = LeaseGate::new(Arc::clone(&mgr));
        let ad = action_def("read_file");
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(tid, &ad, &auto, &params);
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Allow));
    }

    #[tokio::test]
    async fn test_no_lease_denies() {
        let mgr = Arc::new(LeaseManager::new());
        let tid = ThreadId::new();
        // No leases granted

        let gate = LeaseGate::new(Arc::clone(&mgr));
        let ad = action_def("shell");
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(tid, &ad, &auto, &params);
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_expired_lease_denies() {
        let mgr = Arc::new(LeaseManager::new());
        let tid = ThreadId::new();
        let lease = mgr
            .grant(
                tid,
                "tools",
                GrantedActions::Specific(vec!["read_file".into()]),
                None,
                Some(1),
            )
            .await
            .unwrap();
        // Exhaust the lease so it becomes invalid
        mgr.consume_use(lease.id).await.unwrap();

        let gate = LeaseGate::new(Arc::clone(&mgr));
        let ad = action_def("read_file");
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(tid, &ad, &auto, &params);
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_revoked_lease_denies() {
        let mgr = Arc::new(LeaseManager::new());
        let tid = ThreadId::new();
        let lease = mgr
            .grant(
                tid,
                "tools",
                GrantedActions::Specific(vec!["read_file".into()]),
                None,
                None,
            )
            .await
            .unwrap();
        mgr.revoke(lease.id, "test").await;

        let gate = LeaseGate::new(Arc::clone(&mgr));
        let ad = action_def("read_file");
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(tid, &ad, &auto, &params);
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_permissive_gate_allows_everything() {
        let mgr = Arc::new(LeaseManager::new());
        let tid = ThreadId::new();
        // No leases, but permissive mode

        let gate = LeaseGate::permissive(Arc::clone(&mgr));
        let ad = action_def("shell");
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(tid, &ad, &auto, &params);
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Allow));
    }
}
