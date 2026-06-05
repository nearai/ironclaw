//! Canonical per-caller thread-scope resolution.
//!
//! Multi-user WebChat pins each run to its authenticated caller, and the
//! loop host writes that run's thread under `owners/<caller>`. Every
//! subsequent read/write for the run must resolve the SAME owner — both
//! the loop host's thread ports ([`crate::loop_driver_host`]) AND the
//! loop-exit completion-evidence read ([`crate::loop_exit_applier`]).
//!
//! [`ThreadScopeResolver::resolve_for_turn_scope`] is the single definition of
//! that owner/project rewrite rule. Both subsystems resolve through it
//! (`resolve` is its owner-only helper), so the rule cannot drift between them
//! — a second hand-rolled copy silently regressing multi-user isolation is
//! exactly the maintainability hazard this removes.

use ironclaw_threads::ThreadScope;
use ironclaw_turns::{TurnActor, TurnScope, run_profile::LoopRunContext};

/// Canonical scoping rule for per-caller and per-project thread isolation.
pub(crate) struct ThreadScopeResolver;

impl ThreadScopeResolver {
    /// Re-point `base`'s `owner_user_id` at the run's authenticated
    /// `actor`, so each caller's thread I/O is isolated to its own
    /// `owners/<user>` subtree.
    ///
    /// Only rewrites when the base scope is owner-scoped: an owner-less
    /// base (no declared owner) or an actor-less run is returned
    /// unchanged, so single-operator and system flows are untouched.
    pub(crate) fn resolve(base: &ThreadScope, actor: Option<&TurnActor>) -> ThreadScope {
        let mut scope = base.clone();
        if scope.owner_user_id.is_some()
            && let Some(actor) = actor
        {
            scope.owner_user_id = Some(actor.user_id.clone());
        }
        scope
    }

    pub(crate) fn resolve_for_run(
        base: &ThreadScope,
        run_context: &LoopRunContext,
    ) -> Result<ThreadScope, ThreadScopeResolutionError> {
        Self::resolve_for_turn_scope(base, &run_context.scope, run_context.actor())
    }

    pub(crate) fn resolve_for_turn_scope(
        base: &ThreadScope,
        turn_scope: &TurnScope,
        actor: Option<&TurnActor>,
    ) -> Result<ThreadScope, ThreadScopeResolutionError> {
        if base.tenant_id != turn_scope.tenant_id
            || turn_scope.agent_id.as_ref() != Some(&base.agent_id)
        {
            return Err(ThreadScopeResolutionError::ScopeMismatch);
        }
        if base.project_id.is_some() && base.project_id != turn_scope.project_id {
            return Err(ThreadScopeResolutionError::ProjectMismatch);
        }
        let mut scope = if turn_scope.has_explicit_thread_owner() {
            let mut scope = base.clone();
            scope.owner_user_id = turn_scope.explicit_owner_user_id().cloned();
            scope
        } else {
            Self::resolve(base, actor)
        };
        if scope.project_id.is_none() {
            scope.project_id = turn_scope.project_id.clone();
        }
        Ok(scope)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadScopeResolutionError {
    ScopeMismatch,
    ProjectMismatch,
}

impl std::fmt::Display for ThreadScopeResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScopeMismatch => write!(formatter, "thread scope does not match turn scope"),
            Self::ProjectMismatch => {
                write!(
                    formatter,
                    "turn project does not match fixed thread scope project"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};

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

    fn turn_scope(project_id: Option<&str>) -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant").expect("tenant"),
            Some(AgentId::new("agent").expect("agent")),
            project_id.map(|project| ProjectId::new(project).expect("project")),
            ThreadId::new("thread").expect("thread"),
        )
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

    #[test]
    fn resolve_for_turn_scope_uses_turn_project_axis() {
        let base = scope(Some("runtime-owner"));
        let resolved = ThreadScopeResolver::resolve_for_turn_scope(
            &base,
            &turn_scope(Some("project-alpha")),
            Some(&actor("alice")),
        )
        .expect("project-agnostic base accepts turn project");

        assert_eq!(
            resolved.project_id.as_ref().map(|project| project.as_str()),
            Some("project-alpha")
        );
        assert_eq!(
            resolved.owner_user_id.as_ref().map(|user| user.as_str()),
            Some("alice")
        );
    }

    #[test]
    fn resolve_for_turn_scope_rejects_fixed_project_mismatch() {
        let mut base = scope(Some("runtime-owner"));
        base.project_id = Some(ProjectId::new("project-alpha").expect("project"));

        let error = ThreadScopeResolver::resolve_for_turn_scope(
            &base,
            &turn_scope(Some("project-beta")),
            Some(&actor("alice")),
        )
        .expect_err("fixed project base must not be overridden by turn scope");

        assert_eq!(error, ThreadScopeResolutionError::ProjectMismatch);
    }

    #[test]
    fn explicit_turn_owner_overrides_actor_rewrite() {
        let base = scope(Some("runtime-owner"));
        let turn_scope = TurnScope::new_with_owner(
            base.tenant_id.clone(),
            Some(base.agent_id.clone()),
            base.project_id.clone(),
            ironclaw_host_api::ThreadId::new("thread").unwrap(),
            None,
        );

        let resolved =
            ThreadScopeResolver::resolve_for_turn_scope(&base, &turn_scope, Some(&actor("alice")))
                .expect("explicit owner is valid for matching scope");

        assert_eq!(resolved.owner_user_id, None);
    }
}
