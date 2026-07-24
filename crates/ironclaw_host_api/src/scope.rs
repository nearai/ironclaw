//! Execution scope contracts.
//!
//! [`ExecutionContext`] is the authority envelope for one invocation. It ties
//! together identity, tenancy, optional process/thread/mission/project context,
//! runtime/trust class, capability grants, mount view, resource scope, and
//! correlation ID. Every filesystem, resource, secret, network, dispatch, spawn,
//! and audit decision should be traceable back to this context.

use serde::{Deserialize, Serialize};

use crate::{
    AgentId, CapabilitySet, CorrelationId, ExtensionId, HostApiError, InvocationId,
    InvocationOrigin, MissionId, MountView, ProcessId, ProjectId, ResourceScope, RunId,
    RuntimeKind, SystemServiceId, TenantId, ThreadId, TrustClass, UserId,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "id")]
pub enum Principal {
    Tenant(TenantId),
    User(UserId),
    Agent(AgentId),
    Project(ProjectId),
    Mission(MissionId),
    Thread(ThreadId),
    Extension(ExtensionId),
    /// Host runtime internals acting on their own behalf. Never match this as a grantable userland principal.
    HostRuntime,
    /// Named trusted system service, such as heartbeat, routine engine, or migration runner.
    System(SystemServiceId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub invocation_id: InvocationId,
    pub correlation_id: CorrelationId,
    pub process_id: Option<ProcessId>,
    pub parent_process_id: Option<ProcessId>,

    pub tenant_id: TenantId,
    pub user_id: UserId,
    /// Authenticated human actor sealed by trusted ingress/loop orchestration.
    ///
    /// This is intentionally distinct from `user_id`, which identifies the
    /// resource subject. Untrusted and system-created contexts leave it unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticated_actor_user_id: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    /// Prompt-visible run identity for the loop turn-run this invocation
    /// belongs to, sitting between `thread_id` (spans many runs) and
    /// `invocation_id` (one tool call) in the scope cascade.
    ///
    /// Stamped host-side by loop orchestration when it builds the invocation
    /// context (see `invocation_context_from_visible` in
    /// `ironclaw_loop_support`); tool calls within the same run share it.
    /// `None` for non-loop callers (system services, one-shot product
    /// invocations). Policy layers that require "within the current run"
    /// continuity (e.g. coding read-before-edit) key on it; consumers must
    /// treat `None` as its own bucket, never as a wildcard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    /// Authoritative origin of this invocation — where the call came from
    /// (§5.2.1), stamped host-side by the ingress that builds the context:
    /// loop orchestration stamps [`InvocationOrigin::LoopRun`], product
    /// surfaces stamp [`InvocationOrigin::Product`], and the routine/heartbeat
    /// scheduler stamps [`InvocationOrigin::Automation`]. Consumed by the
    /// capability kernel's `authorize()` fold to seal the [`crate::Invocation`]
    /// origin.
    ///
    /// `None` for a context whose ingress has not (yet) stamped an origin; the
    /// kernel falls back to reconstructing [`InvocationOrigin::LoopRun`] from
    /// `run_id` when it is set, so the loop path is covered even before it
    /// stamps `origin` explicitly. It must never be a stand-in placeholder —
    /// an ingress either knows its true origin or leaves this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<InvocationOrigin>,
    pub extension_id: ExtensionId,
    pub runtime: RuntimeKind,
    pub trust: TrustClass,

    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub resource_scope: ResourceScope,
}

impl ExecutionContext {
    /// Build a local/single-user execution context using the canonical default
    /// tenant, agent, and bootstrap project.
    ///
    /// Callers still supply extension/runtime/trust/grants/mounts because those
    /// are product-workflow decisions; this helper only normalizes local scope.
    pub fn local_default(
        user_id: UserId,
        extension_id: ExtensionId,
        runtime: RuntimeKind,
        trust: TrustClass,
        grants: CapabilitySet,
        mounts: MountView,
    ) -> Result<Self, HostApiError> {
        let invocation_id = InvocationId::new();
        let resource_scope = ResourceScope::local_default(user_id.clone(), invocation_id)?;
        let context = Self {
            invocation_id,
            correlation_id: CorrelationId::new(),
            process_id: None,
            parent_process_id: None,
            tenant_id: resource_scope.tenant_id.clone(),
            user_id,
            authenticated_actor_user_id: None,
            agent_id: resource_scope.agent_id.clone(),
            project_id: resource_scope.project_id.clone(),
            mission_id: None,
            thread_id: None,
            run_id: None,
            origin: None,
            extension_id,
            runtime,
            trust,
            grants,
            mounts,
            resource_scope,
        };
        context.validate()?;
        Ok(context)
    }

    /// The authoritative invocation origin (§5.2.1): the ingress-stamped
    /// [`Self::origin`], falling back to reconstructing
    /// [`InvocationOrigin::LoopRun`] from [`Self::run_id`] when a loop ingress
    /// stamped only the run id. `None` for a context carrying neither (a
    /// host-internal or test context). This is the single definition of the
    /// "`run_id` implies a `LoopRun` origin" rule — the capability seal and the
    /// authorization gate both resolve origin through here rather than
    /// re-deriving it.
    ///
    /// # Scheduled runs never downgrade here
    ///
    /// The `run_id` fallback reconstructs [`InvocationOrigin::LoopRun`] **only** —
    /// the mutation-permissive interactive origin. It can never fabricate a
    /// scheduled origin, so the safety of the trigger self-mutation policy
    /// (which denies [`InvocationOrigin::ScheduledLoopRun`]) rests on a single
    /// upstream invariant: **a scheduled ingress must stamp its `origin`
    /// explicitly.** Loop orchestration already does this — `ironclaw_loop_host`'s
    /// `invocation_context_from_visible` stamps `ScheduledLoopRun` for a
    /// `ScheduledTrigger` product context — so a scheduled run always arrives with
    /// `origin = Some(ScheduledLoopRun(..))` and never reaches the fallback below.
    ///
    /// The fallback is deliberately *not* guarded (no `debug_assert!`, no
    /// fail-closed): the `origin = None, run_id = Some` shape is a pinned
    /// transitional-compat contract for legacy interactive contexts (see
    /// `ironclaw_capabilities::host` `authorize_seals_..._real_origin_across_ingresses`),
    /// and it must keep resolving to `LoopRun`. Because a scheduled run is never
    /// un-stamped, tightening this fallback would only reject the safe legacy path
    /// while doing nothing about the (non-occurring) scheduled case. The guard
    /// that matters lives at the ingress that stamps `origin`, not here.
    pub fn resolved_origin(&self) -> Option<InvocationOrigin> {
        self.origin
            .clone()
            .or_else(|| self.run_id.map(InvocationOrigin::LoopRun))
    }

    pub fn validate(&self) -> Result<(), HostApiError> {
        if self.resource_scope.invocation_id != self.invocation_id {
            return Err(HostApiError::invariant(
                "resource_scope.invocation_id must match execution context invocation_id",
            ));
        }
        if self.resource_scope.tenant_id != self.tenant_id {
            return Err(HostApiError::invariant(
                "resource_scope.tenant_id must match execution context tenant_id",
            ));
        }
        if self.resource_scope.user_id != self.user_id {
            return Err(HostApiError::invariant(
                "resource_scope.user_id must match execution context user_id",
            ));
        }
        if self.resource_scope.agent_id != self.agent_id {
            return Err(HostApiError::invariant(
                "resource_scope.agent_id must match execution context agent_id",
            ));
        }
        if self.resource_scope.project_id != self.project_id {
            return Err(HostApiError::invariant(
                "resource_scope.project_id must match execution context project_id",
            ));
        }
        if self.resource_scope.mission_id != self.mission_id {
            return Err(HostApiError::invariant(
                "resource_scope.mission_id must match execution context mission_id",
            ));
        }
        if self.resource_scope.thread_id != self.thread_id {
            return Err(HostApiError::invariant(
                "resource_scope.thread_id must match execution context thread_id",
            ));
        }
        self.mounts.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduled_context() -> ExecutionContext {
        ExecutionContext::local_default(
            UserId::new("scheduled-subject").unwrap(),
            ExtensionId::new("demo").unwrap(),
            RuntimeKind::Script,
            TrustClass::Sandbox,
            CapabilitySet::default(),
            MountView::default(),
        )
        .unwrap()
    }

    #[test]
    fn resolved_origin_preserves_explicit_scheduled_origin_without_downgrade() {
        // The core Item-4 guarantee: a scheduled run resolves to
        // ScheduledLoopRun (which the trigger self-mutation policy denies), never
        // the mutation-permissive LoopRun the run_id fallback would guess.
        let mut context = scheduled_context();
        let run_id = RunId::new();
        context.run_id = Some(run_id);
        context.origin = Some(InvocationOrigin::ScheduledLoopRun(run_id));
        assert_eq!(
            context.resolved_origin(),
            Some(InvocationOrigin::ScheduledLoopRun(run_id)),
        );
    }

    #[test]
    fn resolved_origin_returns_stamped_loop_run() {
        let mut context = scheduled_context();
        let run_id = RunId::new();
        context.run_id = Some(run_id);
        context.origin = Some(InvocationOrigin::LoopRun(run_id));
        assert_eq!(
            context.resolved_origin(),
            Some(InvocationOrigin::LoopRun(run_id)),
        );
    }

    #[test]
    fn resolved_origin_is_none_without_run_or_origin() {
        // A host-internal / one-shot context carries neither: never guessed.
        assert_eq!(scheduled_context().resolved_origin(), None);
    }

    #[test]
    fn resolved_origin_fallback_is_loop_run_only_never_scheduled() {
        // Transitional-compat contract (pinned by `ironclaw_capabilities::host`):
        // an un-stamped context carrying only a run_id resolves to LoopRun. The
        // load-bearing safety property is that this fallback can ONLY ever be the
        // mutation-permissive LoopRun — it never fabricates a ScheduledLoopRun —
        // so a scheduled run, which always stamps its origin upstream, cannot be
        // recovered (or downgraded) through this path.
        let mut context = scheduled_context();
        let run_id = RunId::new();
        context.run_id = Some(run_id);
        // origin intentionally left unset.
        assert_eq!(
            context.resolved_origin(),
            Some(InvocationOrigin::LoopRun(run_id)),
        );
    }

    #[test]
    fn legacy_execution_context_without_optional_identity_fields_deserializes() {
        let mut context = ExecutionContext::local_default(
            UserId::new("subject").unwrap(),
            ExtensionId::new("demo").unwrap(),
            RuntimeKind::Script,
            TrustClass::Sandbox,
            CapabilitySet::default(),
            MountView::default(),
        )
        .unwrap();
        context.authenticated_actor_user_id = Some(UserId::new("slack-alice").unwrap());
        context.run_id = Some(RunId::new());
        let mut legacy = serde_json::to_value(context).unwrap();
        let fields = legacy.as_object_mut().unwrap();
        fields.remove("authenticated_actor_user_id");
        fields.remove("run_id");

        let decoded: ExecutionContext = serde_json::from_value(legacy).unwrap();

        assert_eq!(decoded.authenticated_actor_user_id, None);
        assert_eq!(decoded.run_id, None);
    }
}
