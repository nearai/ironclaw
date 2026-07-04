//! Per-capability preset constructors for [`RebornIntegrationGroup`] /
//! [`RebornIntegrationGroupBuilder`].
//!
//! `group.rs` owns the one-shared-runtime assembly mechanics
//! (`RebornIntegrationGroupBuilder::build_base` / `into_group`); this file is
//! a private child module of `group` (declared `#[path = "group_constructors.rs"]
//! mod group_constructors;` in `group.rs`, NOT `pub mod` from `mod.rs`) that
//! catalogs "which capability" selections layered on top of that mechanics —
//! one method per `HostRuntimeCapabilityHarness` preset. Keeping it a child
//! module (rather than a top-level sibling) lets it reach `build_base`/
//! `into_group`/`GroupBaseData` at plain module-private visibility instead of
//! widening them to `pub(crate)` for the whole test-support crate. Split out
//! (design precedent: `harness_mcp.rs`) once `group.rs` crossed the 1000-line
//! ceiling with PR-E2's E-SKILL/E-DURABLE/E-GATEWAY additions; new capability
//! presets belong HERE, not back in `group.rs`.

// Shared by all group test binaries; symbols read as dead when a binary does
// not exercise every preset (mirrors the same attribute on `group.rs`/`builder.rs`).
#![allow(dead_code)]

use std::sync::Arc;

use super::super::harness::HostRuntimeCapabilityHarness;
use super::{
    GroupCapability, HarnessResult, RebornIntegrationGroup, RebornIntegrationGroupBuilder,
};

impl RebornIntegrationGroup {
    /// Group with real file-tool approval stores (write_file/read_file at
    /// `PermissionMode::Ask`). Auto-approve is disabled for the group scope at
    /// construction so gated tool calls raise real `BlockedApproval` gates.
    /// Resolve with `approve_gate`/`deny_gate` per thread; re-enable with
    /// `enable_auto_approve` for the no-gate arm.
    pub async fn live_approvals() -> HarnessResult<Self> {
        Self::builder().live_approvals().await
    }

    /// Group with core built-in tools (memory/http/echo/time/json/shell).
    /// Auto-approve is enabled for all capability ids in the group scope.
    pub async fn builtin_tools() -> HarnessResult<Self> {
        Self::builder().builtin_tools().await
    }

    /// Group with extension-lifecycle tools
    /// (extension_search/install/activate/remove). Auto-approve is enabled;
    /// registry credentials are seeded.
    pub async fn extension_lifecycle() -> HarnessResult<Self> {
        Self::builder().extension_lifecycle().await
    }

    /// Group whose GitHub extension's credential account resolves to
    /// `AuthRequired`, so a scripted `github.*` tool call raises a real
    /// `TurnStatus::BlockedAuth` gate (E-AUTHGATE seam). Drive with
    /// `submit_turn_until_auth_blocked`.
    pub async fn live_auth_gate() -> HarnessResult<Self> {
        Self::builder().live_auth_gate().await
    }

    /// C-JOURNEY convergence seam: group surfacing BOTH an unseeded GitHub
    /// capability (raises `TurnStatus::BlockedAuth`, resolvable with
    /// `resolve_auth_gate`/`deny_auth_gate`) AND real file-tool approval
    /// stores (`write_file`/`read_file` at `PermissionMode::Ask`, raises
    /// `TurnStatus::BlockedApproval`, resolvable with `approve_gate`/`deny_gate`)
    /// on the SAME `build_reborn_services` runtime — unlike `live_auth_gate`
    /// (a separate, lower-level `HostRuntimeServices` build with a hardcoded
    /// credential resolver and no run_state store), this group's auth gate
    /// resolves through the REAL `ProductAuthRuntimeCredentialResolver`, so
    /// `resolve_auth_gate`'s happy-path resume actually completes. Auto-approve
    /// is disabled for the group scope at construction so gated file-tool calls
    /// raise real `BlockedApproval` gates.
    pub async fn live_auth_and_approval() -> HarnessResult<Self> {
        Self::builder().live_auth_and_approval().await
    }

    /// Group with the local-dev synthetic `project_create` capability wired
    /// (E-PROJ seam). Auto-approve is enabled.
    pub async fn project_lifecycle() -> HarnessResult<Self> {
        Self::builder().project_lifecycle().await
    }

    /// C-SYNTH `project_create` fault-injection arm: same surface as
    /// `project_lifecycle()`, but the underlying `ProjectService` is wrapped
    /// in `FaultInjectingProjectService`
    /// (`HostRuntimeCapabilityHarness::project_tools_with_fault_injection`)
    /// so a `create_project` naming
    /// `FAULT_INJECT_DENIED_PROJECT_NAME` surfaces a real
    /// `ProjectServiceError::Denied` through the actual capability
    /// dispatch instead of only at the `project_service_outcome` unit level.
    pub async fn project_lifecycle_fault_injected() -> HarnessResult<Self> {
        Self::builder().project_lifecycle_fault_injected().await
    }

    /// Group whose ONLY capability is `builtin.profile_set` (E-PROFILE seam).
    /// Auto-approve is enabled. Use `user_profile_source_for_test()` to read
    /// a written profile back through the same adapter the group's planned
    /// runtime resolves user profiles from.
    pub async fn profile_tools() -> HarnessResult<Self> {
        Self::builder().profile_tools().await
    }

    /// Group with trigger-management tools
    /// (trigger_create/list/pause/resume/remove). Auto-approve is enabled for
    /// all capability ids in the group scope so the `Ask`-mode verbs dispatch
    /// through the real capability path instead of raising approval gates.
    pub async fn triggers() -> HarnessResult<Self> {
        Self::builder().triggers().await
    }

    /// Group whose ONLY capability is `builtin.skill_activate` (E-SKILL seam).
    /// A system-scoped `greet` skill is seeded for the run; a scripted
    /// `builtin.skill_activate` call for `greet` dispatches the synthetic
    /// capability and injects the skill's instructions into the model
    /// request through the runtime's `skill_context_source`. Auto-approve is
    /// enabled.
    pub async fn skill_activation_tools() -> HarnessResult<Self> {
        Self::builder().skill_activation_tools().await
    }

    /// C-MULTIUSER: core built-in tools (memory/http/shell/…) with **per-actor
    /// capability scoping** (`with_run_owner_scoped_capability_dispatch`). Each
    /// thread dispatches its capabilities under its OWN run owner's
    /// `(tenant, user)` scope, so `memory_write`/`read`/`search` resolve to that
    /// owner's `/memory/tenants/<t>/users/<u>/…` subtree — actor A's memory is
    /// invisible to actor B. Distinct from [`builtin_tools`], which collapses
    /// every actor onto one fixed capability user (shared memory). Drives
    /// `scenario_memory_isolation_across_actors`.
    pub async fn multiuser_memory_tools() -> HarnessResult<Self> {
        Self::builder().multiuser_memory_tools().await
    }

    /// C-MULTIUSER: file-approval tools (write_file/read_file @ `Ask`) with
    /// **per-actor capability scoping**. A grant via
    /// [`RebornIntegrationGroup::enable_auto_approve_for_owner`] and an explicit
    /// OFF via [`RebornIntegrationGroup::disable_auto_approve_for_owner`] each
    /// apply to that owner ALONE. Drives
    /// `scenario_auto_approve_isolation_across_actors`: actor A's always-allow
    /// grant lets A's call complete gate-free while actor B (set OFF) still
    /// raises a real `BlockedApproval` gate on the identical call.
    pub async fn multiuser_approvals() -> HarnessResult<Self> {
        Self::builder().multiuser_approvals().await
    }

    /// Group surfacing the two synthetic `outbound_delivery_*` capabilities over
    /// an injected facade double (C-SYNTH outbound seam). `target_set` requires
    /// approval; global auto-approve defaults ON so the happy/`NotFound` arms
    /// dispatch through `Allow`. The approval-gate arm disables auto-approve
    /// per-test with `disable_auto_approve`; the deny arm persists a `Disabled`
    /// tool override via `capability_harness().disable_outbound_target_set_tool`.
    pub async fn outbound_target_tools() -> HarnessResult<Self> {
        Self::builder().outbound_target_tools().await
    }

    /// Group with the skill-management verbs (`skill_list`/`skill_install`/
    /// `skill_remove`) at int tier (C-SKILL). Previously covered ONLY at the
    /// QA/trace tier (`with_host_runtime_skill_management_capabilities`,
    /// `harness.rs`); this reuses the SAME
    /// `HostRuntimeCapabilityHarness::skill_management_tools()` preset over
    /// the real turn → capability path. Auto-approve is enabled.
    pub async fn skill_management_tools() -> HarnessResult<Self> {
        Self::builder().skill_management_tools().await
    }

    /// Group with the attachment read port + inbound lander wired (C-ATTACH
    /// seam), no first-party capability dispatch. Use
    /// [`RebornThreadBuilder::with_model_override`] to route a thread through a
    /// vision-capable model id and
    /// [`RebornIntegrationHarness::submit_turn_with_image_attachment`] to land
    /// an image and submit it in one turn.
    pub async fn attachment_tools() -> HarnessResult<Self> {
        Self::builder().attachment_tools().await
    }
}

impl RebornIntegrationGroupBuilder {
    /// Build a `RebornIntegrationGroup` for an already-selected `GroupCapability`.
    /// Shared tail of the constructors whose capability is independent of the
    /// resolved base (`builtin_tools`/`extension_lifecycle`) and the degenerate
    /// single-shot path (`RebornIntegrationHarnessBuilder::build`).
    /// `live_approvals` and `profile_tools` both resolve their capability's
    /// executor user FROM `base` (`canonical_subject_user()`), so they call
    /// `build_base` + `into_group` directly instead, reusing the SAME `base`
    /// for both the user lookup and the group assembly.
    pub(crate) async fn build_with_capability(
        self,
        capability: GroupCapability,
    ) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        self.into_group(base, capability).await
    }

    /// Build a live-approvals group. See [`RebornIntegrationGroup::live_approvals`].
    pub async fn live_approvals(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        // Execute first-party tools under the run's CANONICAL binding subject
        // user (the hashed `UserId` the actor `host-user` resolves to), not the
        // constructor's fixed test user, so capability dispatch, approval
        // persistence, auto-approve keying, and gate-evidence lookup all share the
        // run's `(tenant, user)` — matching production. Reuse the SAME canonical
        // binding `build_base` already resolved for the shared turn-store /
        // evidence scope, so the approval user and the turn-store scope are
        // derived from one probe and cannot drift.
        let subject_user = base.canonical_subject_user()?;
        let host_runtime = HostRuntimeCapabilityHarness::file_tools_requiring_approval()
            .await?
            .with_user_id(subject_user);
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        let group = self.into_group(base, capability).await?;
        // Disable auto-approve once at build time so every thread in this group
        // faces real approval gates. The dispatch-time check is keyed on the
        // capability harness's executor user (NOT the binding owner), so target
        // `auto_approve_scope()` — `(run tenant, capability user)`.
        // `live_approvals` always constructs `GroupCapability::HostRuntime`, so
        // both `auto_approve_scope()` and `capability_harness()` are guaranteed
        // `Some` — use `expect` rather than a redundant `if let`.
        let scope = group
            .shared
            .auto_approve_scope()
            .expect("live_approvals always uses HostRuntime; scope is always Some");
        let arc = group
            .capability_harness()
            .expect("live_approvals always uses HostRuntime");
        arc.disable_global_auto_approve(scope).await?;
        Ok(group)
    }

    /// Build a core built-in tools group. See [`RebornIntegrationGroup::builtin_tools`].
    pub async fn builtin_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = HostRuntimeCapabilityHarness::core_builtin_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build an extension-lifecycle group. See [`RebornIntegrationGroup::extension_lifecycle`].
    pub async fn extension_lifecycle(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = HostRuntimeCapabilityHarness::extension_lifecycle_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build an auth-gate group. See [`RebornIntegrationGroup::live_auth_gate`].
    ///
    /// No auto-approve disable and no approval-gate evidence: auth gates are
    /// self-evidencing via the BeforeBlock checkpoint (loop_exit_applier.rs). Do
    /// NOT add approval-gate evidence here — that store is only for approval gates.
    pub async fn live_auth_gate(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = HostRuntimeCapabilityHarness::github_issue_tools_auth_required().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build an auth+approval convergence group. See
    /// [`RebornIntegrationGroup::live_auth_and_approval`]. Cannot go through
    /// `build_with_capability` — like `live_approvals`, the capability's
    /// executor user must be aligned to `base`'s canonical binding subject
    /// user (`.with_user_id`) so dispatch-time capability resolution
    /// (approval persistence AND the seeded GitHub credential account's
    /// visibility scope) matches the run's actual `(tenant, user)`, not the
    /// constructor's fixed test user.
    pub async fn live_auth_and_approval(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        let subject_user = base.canonical_subject_user()?;
        let host_runtime = HostRuntimeCapabilityHarness::file_and_github_auth_tools()
            .await?
            .with_user_id(subject_user);
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        let group = self.into_group(base, capability).await?;
        // `file_and_github_auth_tools` already disabled auto-approve under its
        // OWN fixed constructor user (before `.with_user_id` reassigned it);
        // disable it again under the run's REAL capability scope, mirroring
        // `live_approvals`'s alignment above.
        let scope = group
            .shared
            .auto_approve_scope()
            .expect("live_auth_and_approval always uses HostRuntime; scope is always Some");
        let arc = group
            .capability_harness()
            .expect("live_auth_and_approval always uses HostRuntime");
        arc.disable_global_auto_approve(scope).await?;
        Ok(group)
    }

    /// Build a project-lifecycle group. See [`RebornIntegrationGroup::project_lifecycle`].
    pub async fn project_lifecycle(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = HostRuntimeCapabilityHarness::project_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a project-lifecycle group with the `project_create`
    /// fault-injection arm wired. See
    /// [`RebornIntegrationGroup::project_lifecycle_fault_injected`].
    pub async fn project_lifecycle_fault_injected(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime =
            HostRuntimeCapabilityHarness::project_tools_with_fault_injection().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a profile-tools group. See [`RebornIntegrationGroup::profile_tools`].
    pub async fn profile_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        // Execute `builtin.profile_set` under the run's canonical binding
        // subject user, mirroring `live_approvals`'s alignment above.
        // Without this, a second thread's loop resolves the profile under
        // the canonical subject user while the write dispatched under the
        // fixed constructor user, so the read-back never sees it. Also why
        // this cannot go through `build_with_capability`: the capability
        // depends on `base`, so `base` must be resolved first.
        let subject_user = base.canonical_subject_user()?;
        let host_runtime = HostRuntimeCapabilityHarness::profile_tools()
            .await?
            .with_user_id(subject_user);
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build a trigger-management group. See [`RebornIntegrationGroup::triggers`].
    pub async fn triggers(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = HostRuntimeCapabilityHarness::trigger_management_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a skill-activation group. See
    /// [`RebornIntegrationGroup::skill_activation_tools`]. Seeds a `greet` system
    /// skill BEFORE `into_group` so the runtime's `skill_context_source` (and the
    /// `skill_activate` capability's `activate_skills_for_run`) resolve it at
    /// activation time. A system skill is used so resolution is independent of the
    /// run's scope owner — the seam only needs the skill to exist.
    pub async fn skill_activation_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        // Pass the group's ACTUAL run-scope tenant (resolved by `build_base`
        // above) rather than a separately hardcoded literal, so the E-SKILL
        // skill context source is built for the same tenant the turn runs
        // under — see `HostRuntimeCapabilityHarness::skill_activation_tools`.
        let host_runtime =
            HostRuntimeCapabilityHarness::skill_activation_tools(&base.canonical_binding.tenant_id)
                .await?;
        host_runtime.seed_system_skill_for_test(
            "greet",
            "greets the user warmly",
            "GREET_SKILL_PROMPT_SENTINEL",
        )?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build a per-actor-scoped memory group.
    /// See [`RebornIntegrationGroup::multiuser_memory_tools`]. Same capability
    /// surface as [`builtin_tools`] but with per-actor capability dispatch, so
    /// each actor's memory lands under its own owner subtree. Self-contained
    /// (no shared helper) so it relocates trivially if the group constructors
    /// are later split out.
    pub async fn multiuser_memory_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = HostRuntimeCapabilityHarness::core_builtin_tools()
            .await?
            .with_run_owner_scoped_capability_dispatch();
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a per-actor-scoped file-approval group.
    /// See [`RebornIntegrationGroup::multiuser_approvals`]. Real approval stores
    /// (write_file/read_file @ `Ask`) plus per-actor capability dispatch. Auto-
    /// approve defaults ON per owner (`AUTO_APPROVE_DEFAULT_ENABLED = true`), so
    /// a scenario that needs an owner to GATE sets that owner OFF explicitly via
    /// `disable_auto_approve_for_owner` (and grants another owner via
    /// `enable_auto_approve_for_owner`) — the per-user setting is what the test
    /// asserts isolates. Because the seam makes the dispatch user equal the turn
    /// owner, a raised approval request persists under — and its gate-evidence
    /// lookup resolves through — the SAME owner, so the gate is verified (not
    /// masked to `Failed`). Self-contained for trivial relocation.
    pub async fn multiuser_approvals(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        let host_runtime = HostRuntimeCapabilityHarness::file_tools_requiring_approval()
            .await?
            .with_run_owner_scoped_capability_dispatch();
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build an outbound-target-tools group. See
    /// [`RebornIntegrationGroup::outbound_target_tools`]. Mirrors `live_approvals`
    /// / `profile_tools`: the synthetic `outbound_delivery_*` capabilities run
    /// under the run's CANONICAL binding subject user (via `with_user_id`), so
    /// the dispatch-time settings/auto-approve scope
    /// (`_shared.auto_approve_scope()` = `(tenant, capability user)`) aligns with
    /// the run's effective dispatch user — the approval-gate arm's
    /// `disable_auto_approve` and the deny arm's `disable_outbound_target_set_tool`
    /// both target that exact `(tenant, user)`. Auto-approve is left at its
    /// default-ON state (no disable here); the gate arm disables it per-test.
    pub async fn outbound_target_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        let subject_user = base.canonical_subject_user()?;
        let host_runtime = HostRuntimeCapabilityHarness::outbound_target_tools()
            .await?
            .with_user_id(subject_user);
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build a skill-management group. See
    /// [`RebornIntegrationGroup::skill_management_tools`].
    pub async fn skill_management_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = HostRuntimeCapabilityHarness::skill_management_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build an attachment-tools group. See [`RebornIntegrationGroup::attachment_tools`].
    pub async fn attachment_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = HostRuntimeCapabilityHarness::attachment_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }
}
