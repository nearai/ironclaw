//! Fire-time trigger access: the check contract, and the checkers that are
//! pure trigger-scope policy.
//!
//! Trigger-fire authorization is not a persisted parallel access table (it
//! replaced `ironclaw_runner::local_trigger_access`, arch-simplification §4.4).
//! It is a decision about *this crate's own noun* — may the user who created a
//! persisted trigger still fire it for the exact tenant/agent/project scope
//! stored on it — so the contract and the scope comparison live beside the
//! trigger record and the worker that consults them, not in the assembly root
//! (CHECKLIST WS6, PROPOSAL §6.10.1: "approval/authorization/trigger-fire
//! policy → … `triggers`").
//!
//! Two things deliberately stay in `ironclaw_reborn_composition`:
//!
//! - **`TriggerFireAccessPolicy`/`TriggerFireAccessGrant`** — the deployment
//!   config value the `serve`/`run` edge resolves and the build turns into a
//!   checker. §6.10.1's Keeps list names "deployment config-as-data" as
//!   composition's charter, and this is that.
//! - **The identity-directory checker** — resolving tenant membership at fire
//!   time is a lookup against a backend composition selects
//!   (`RebornUserDirectory`); an adapter over a chosen backend is assembly, and
//!   moving it here would buy this crate a dependency on the identity crate to
//!   hold one `get_user` call.
//!
//! What is here is the part with no backend at all: the request/decision
//! vocabulary, the exact-scope comparison, and the OR-combinator.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    Timestamp,
    ids::{AgentId, ProjectId, TenantId, UserId},
};

use crate::TriggerId;

const DENY_REASON: &str = "trigger creator does not have active access for this scope";

/// Fire-time access request for a persisted trigger.
///
/// Checks are exact: `None` for `agent_id` or `project_id` means the trigger
/// has no value for that scope dimension, not that the checker should treat it
/// as a wildcard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerFireAccessCheck {
    /// Tenant that owns the persisted trigger.
    pub tenant_id: TenantId,
    /// User that created the persisted trigger and whose access is evaluated
    /// again at fire time.
    pub creator_user_id: UserId,
    /// Optional agent scope stored on the trigger.
    pub agent_id: Option<AgentId>,
    /// Optional project scope stored on the trigger.
    pub project_id: Option<ProjectId>,
    /// Trigger being fired. Included so production access checks can audit or
    /// apply trigger-specific policy without changing this request shape.
    pub trigger_id: TriggerId,
    /// Deterministic fire slot being submitted. Included for audit and policy
    /// decisions that depend on scheduled fire identity.
    pub fire_slot: Timestamp,
}

/// Result of a fire-time trigger access check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerFireAccessDecision {
    /// The trigger creator is still authorized for the exact trigger scope.
    Allowed,
    /// The trigger creator is not authorized for the exact trigger scope.
    Denied { reason: String },
}

/// Error returned when the access backend cannot answer the request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TriggerFireAccessError {
    /// The backing access source was unavailable; trigger fire handling should
    /// treat this as retryable rather than a permanent denial.
    #[error("trigger fire access backend unavailable: {reason}")]
    Unavailable { reason: String },
}

/// Fire-time trigger access checker. The composition root selects and wires the
/// implementations from its deployment policy.
#[async_trait]
pub trait TriggerFireAccessChecker: Send + Sync {
    /// Check whether the persisted trigger creator may fire the trigger for
    /// the exact stored tenant/agent/project scope.
    async fn check_trigger_fire_access(
        &self,
        request: TriggerFireAccessCheck,
    ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError>;
}

/// Does the fire-time check's exact scope match the granted `(agent, project)`
/// grant? Scope is exact — `None` project means "no project", never a wildcard
/// (matches [`TriggerFireAccessCheck`] semantics).
fn scope_matches(
    check: &TriggerFireAccessCheck,
    agent: &AgentId,
    project: &Option<ProjectId>,
) -> bool {
    check.agent_id.as_ref() == Some(agent) && &check.project_id == project
}

/// Deny with this crate's single fire-access denial reason. Public so the
/// composition-owned identity-directory checker renders the same string as the
/// checkers here — the reason is trigger policy, not per-adapter wording.
pub fn trigger_fire_access_denied() -> TriggerFireAccessDecision {
    TriggerFireAccessDecision::Denied {
        reason: DENY_REASON.to_string(),
    }
}

/// Does this check's scope match the granted `(agent, project)` pair? Exposed
/// for the same reason as [`trigger_fire_access_denied`]: the exact-scope rule
/// is trigger policy and every checker must apply the identical one.
pub fn trigger_fire_scope_matches(
    check: &TriggerFireAccessCheck,
    agent: &AgentId,
    project: &Option<ProjectId>,
) -> bool {
    scope_matches(check, agent, project)
}

/// A single configured owner may fire triggers for one exact scope — the
/// env-token `serve` and CLI `run` owner grant. Pure comparison, no I/O.
///
/// The `tenant_id` bound is load-bearing: the due-trigger repository is global,
/// so a fire-time check that matched only owner + scope could authorize a
/// foreign tenant's trigger whose creator id happened to equal this owner. The
/// former store keyed every row on tenant; this preserves that.
pub struct StaticOwnerTriggerFireChecker {
    tenant_id: TenantId,
    owner: UserId,
    agent: AgentId,
    project: Option<ProjectId>,
}

impl StaticOwnerTriggerFireChecker {
    pub fn new(
        tenant_id: TenantId,
        owner: UserId,
        agent: AgentId,
        project: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            owner,
            agent,
            project,
        }
    }
}

#[async_trait]
impl TriggerFireAccessChecker for StaticOwnerTriggerFireChecker {
    async fn check_trigger_fire_access(
        &self,
        request: TriggerFireAccessCheck,
    ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError> {
        let allowed = request.tenant_id == self.tenant_id
            && request.creator_user_id == self.owner
            && scope_matches(&request, &self.agent, &self.project);
        Ok(if allowed {
            TriggerFireAccessDecision::Allowed
        } else {
            trigger_fire_access_denied()
        })
    }
}

/// OR-combines several checkers: `Allowed` if any grant allows; otherwise
/// `Unavailable` if any grant's backend was unavailable (retryable, so a
/// transient identity-store fault is not a hard denial); otherwise `Denied`.
pub struct CompositeTriggerFireChecker {
    checkers: Vec<Arc<dyn TriggerFireAccessChecker>>,
}

impl CompositeTriggerFireChecker {
    pub fn new(checkers: Vec<Arc<dyn TriggerFireAccessChecker>>) -> Self {
        Self { checkers }
    }
}

#[async_trait]
impl TriggerFireAccessChecker for CompositeTriggerFireChecker {
    async fn check_trigger_fire_access(
        &self,
        request: TriggerFireAccessCheck,
    ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError> {
        // Split so the last checker takes `request` by move — no redundant
        // final clone (the common case is a single StaticOwner + SsoMembership
        // pair, so this saves one clone per fire).
        let Some((last, rest)) = self.checkers.split_last() else {
            return Ok(trigger_fire_access_denied());
        };
        let mut unavailable: Option<TriggerFireAccessError> = None;
        for checker in rest {
            match checker.check_trigger_fire_access(request.clone()).await {
                Ok(TriggerFireAccessDecision::Allowed) => {
                    return Ok(TriggerFireAccessDecision::Allowed);
                }
                Ok(TriggerFireAccessDecision::Denied { .. }) => {}
                Err(error) => unavailable = Some(error),
            }
        }
        match last.check_trigger_fire_access(request).await {
            Ok(TriggerFireAccessDecision::Allowed) => Ok(TriggerFireAccessDecision::Allowed),
            Ok(TriggerFireAccessDecision::Denied { .. }) => match unavailable {
                Some(error) => Err(error),
                None => Ok(trigger_fire_access_denied()),
            },
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(creator: &str, agent: Option<&str>, project: Option<&str>) -> TriggerFireAccessCheck {
        TriggerFireAccessCheck {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            creator_user_id: UserId::new(creator).expect("user"),
            agent_id: agent.map(|a| AgentId::new(a).expect("agent")),
            project_id: project.map(|p| ProjectId::new(p).expect("project")),
            trigger_id: TriggerId::new(),
            fire_slot: chrono::Utc::now(),
        }
    }

    fn static_checker() -> StaticOwnerTriggerFireChecker {
        StaticOwnerTriggerFireChecker::new(
            TenantId::new("tenant").expect("tenant"),
            UserId::new("owner").expect("user"),
            AgentId::new("agent").expect("agent"),
            Some(ProjectId::new("project").expect("project")),
        )
    }

    #[tokio::test]
    async fn static_owner_allows_exact_owner_and_scope() {
        let decision = static_checker()
            .check_trigger_fire_access(check("owner", Some("agent"), Some("project")))
            .await
            .expect("check");
        assert_eq!(decision, TriggerFireAccessDecision::Allowed);
    }

    #[tokio::test]
    async fn static_owner_denies_non_owner() {
        let decision = static_checker()
            .check_trigger_fire_access(check("intruder", Some("agent"), Some("project")))
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
    }

    #[tokio::test]
    async fn static_owner_denies_scope_mismatch() {
        // Right owner, wrong project scope.
        let decision = static_checker()
            .check_trigger_fire_access(check("owner", Some("agent"), Some("other")))
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
        // Right owner, missing project where one was granted.
        let decision = static_checker()
            .check_trigger_fire_access(check("owner", Some("agent"), None))
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
    }

    #[tokio::test]
    async fn static_owner_denies_foreign_tenant() {
        // The due-trigger repository is global: a foreign tenant's trigger with
        // a matching owner id + scope must NOT be authorized (regression guard).
        let foreign = TriggerFireAccessCheck {
            tenant_id: TenantId::new("other-tenant").expect("tenant"),
            creator_user_id: UserId::new("owner").expect("user"),
            agent_id: Some(AgentId::new("agent").expect("agent")),
            project_id: Some(ProjectId::new("project").expect("project")),
            trigger_id: TriggerId::new(),
            fire_slot: chrono::Utc::now(),
        };
        let decision = static_checker()
            .check_trigger_fire_access(foreign)
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
    }

    #[tokio::test]
    async fn composite_allows_if_any_grant_allows() {
        // Two static owners; only the second matches the creator.
        let checkers: Vec<Arc<dyn TriggerFireAccessChecker>> = vec![
            Arc::new(StaticOwnerTriggerFireChecker::new(
                TenantId::new("tenant").expect("tenant"),
                UserId::new("owner-a").expect("user"),
                AgentId::new("agent").expect("agent"),
                Some(ProjectId::new("project").expect("project")),
            )),
            Arc::new(StaticOwnerTriggerFireChecker::new(
                TenantId::new("tenant").expect("tenant"),
                UserId::new("owner-b").expect("user"),
                AgentId::new("agent").expect("agent"),
                Some(ProjectId::new("project").expect("project")),
            )),
        ];
        let composite = CompositeTriggerFireChecker::new(checkers);
        let decision = composite
            .check_trigger_fire_access(check("owner-b", Some("agent"), Some("project")))
            .await
            .expect("check");
        assert_eq!(decision, TriggerFireAccessDecision::Allowed);
    }

    #[tokio::test]
    async fn composite_denies_if_no_grant_allows() {
        let checkers: Vec<Arc<dyn TriggerFireAccessChecker>> =
            vec![Arc::new(StaticOwnerTriggerFireChecker::new(
                TenantId::new("tenant").expect("tenant"),
                UserId::new("owner-a").expect("user"),
                AgentId::new("agent").expect("agent"),
                None,
            ))];
        let composite = CompositeTriggerFireChecker::new(checkers);
        let decision = composite
            .check_trigger_fire_access(check("stranger", Some("agent"), None))
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
    }
}
