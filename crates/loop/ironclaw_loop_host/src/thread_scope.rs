//! Canonical per-caller thread-scope resolution.
//!
//! Multi-user WebChat pins each run to its authenticated caller, and the
//! loop host writes that run's thread under `owners/<caller>`. Every
//! subsequent read/write for the run must resolve the SAME owner — the
//! loop host's thread ports, runner completion-evidence reads, and any
//! composition-side durable thread append that is keyed by a
//! [`LoopRunContext`](ironclaw_loop_contracts::LoopRunContext).
//!
//! [`ThreadScopeResolver::resolve`] is the single definition of that
//! owner-rewrite rule. Both subsystems call it, so the rule cannot drift
//! between them — a second hand-rolled copy silently regressing
//! multi-user isolation is exactly the maintainability hazard this
//! removes.

use ironclaw_threads::ThreadScope;
use ironclaw_turns::{TurnActor, TurnScope};

/// Canonical owner-scoping rule for per-caller thread isolation.
pub struct ThreadScopeResolver;

impl ThreadScopeResolver {
    /// Re-point `base`'s `owner_user_id` at the run's authenticated
    /// `actor`, so each caller's thread I/O is isolated to its own
    /// `owners/<user>` subtree.
    ///
    /// Only rewrites when the base scope is owner-scoped: an owner-less
    /// base (no declared owner) or an actor-less run is returned
    /// unchanged, so single-operator and system flows are untouched.
    pub fn resolve(base: &ThreadScope, actor: Option<&TurnActor>) -> ThreadScope {
        let mut scope = base.clone();
        if scope.owner_user_id.is_some()
            && let Some(actor) = actor
        {
            scope.owner_user_id = Some(actor.user_id.clone());
        }
        scope
    }

    /// Resolve the run's thread owner: an explicit turn owner (host/trigger
    /// creator, subagent parent→child propagation, or Ownerless→system) is
    /// authoritative; otherwise the owner follows the run's actor (multi-user
    /// WebChat). Owner == actor since the ephemeral-per-ping remodel, so the
    /// explicit-owner and actor branches never disagree — this is run-user
    /// resolution, not an owner-vs-actor choice.
    pub fn resolve_for_turn(
        base: &ThreadScope,
        turn_scope: &TurnScope,
        actor: Option<&TurnActor>,
    ) -> ThreadScope {
        if turn_scope.has_explicit_thread_owner() {
            let mut scope = base.clone();
            scope.owner_user_id = turn_scope.explicit_owner_user_id().cloned();
            return scope;
        }
        Self::resolve(base, actor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{AgentId, TenantId, UserId};

    fn scope(owner: Option<&str>) -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            agent_id: AgentId::new("agent").expect("agent"),
            project_id: None,
            owner_user_id: owner.map(|o| UserId::new(o).expect("user")),
            mission_id: None,
        }
    }

    fn actor(user: &str) -> TurnActor {
        TurnActor::new(UserId::new(user).expect("user"))
    }

    #[test]
    fn rewrites_owner_to_run_actor_when_base_is_owner_scoped() {
        let base = scope(Some("runtime-owner"));
        let a = ThreadScopeResolver::resolve(&base, Some(&actor("alice")));
        let b = ThreadScopeResolver::resolve(&base, Some(&actor("bob")));
        assert_eq!(a.owner_user_id.as_ref().map(|u| u.as_str()), Some("alice"));
        assert_eq!(b.owner_user_id.as_ref().map(|u| u.as_str()), Some("bob"));
    }

    #[test]
    fn leaves_owner_unchanged_when_run_has_no_actor() {
        let base = scope(Some("runtime-owner"));
        let resolved = ThreadScopeResolver::resolve(&base, None);
        assert_eq!(
            resolved.owner_user_id.as_ref().map(|u| u.as_str()),
            Some("runtime-owner"),
        );
    }

    #[test]
    fn leaves_owner_less_base_unchanged_even_with_an_actor() {
        let base = scope(None);
        let resolved = ThreadScopeResolver::resolve(&base, Some(&actor("alice")));
        assert!(
            resolved.owner_user_id.is_none(),
            "an owner-agnostic base must stay system/shared-scoped"
        );
    }

    /// An explicit turn owner is applied verbatim, skipping the actor rewrite.
    /// Here an Ownerless turn scope stays system-scoped even with an actor
    /// present — preserving Ownerless→SYSTEM. (Explicit-owner turns are
    /// triggers and subagent children; owner==actor for actor-fallback WebChat
    /// runs, so the only "explicit over the actor" case left is this
    /// ownerless/system one.)
    #[test]
    fn explicit_ownerless_turn_scope_stays_system_over_the_actor_rewrite() {
        let base = scope(Some("runtime-owner"));
        let turn_scope = TurnScope::new_with_owner(
            base.tenant_id.clone(),
            Some(base.agent_id.clone()),
            base.project_id.clone(),
            ironclaw_host_api::ids::ThreadId::new("thread").unwrap(),
            None,
        );

        let resolved =
            ThreadScopeResolver::resolve_for_turn(&base, &turn_scope, Some(&actor("alice")));

        assert_eq!(resolved.owner_user_id, None);
    }
}
