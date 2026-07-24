//! Per-capability preset constructors for [`RebornIntegrationGroup`] /
//! [`RebornIntegrationGroupBuilder`] — one method per `HostRuntimeCapabilityHarness`
//! preset. Private child module of `group.rs` (owns the shared assembly
//! mechanics: `build_base`/`into_group`), so it reaches those + `GroupBaseData`
//! at module-private visibility instead of widening them to `pub(crate)` for
//! the whole test-support crate. New capability presets belong HERE.

// Shared by all group test binaries; symbols read as dead when a binary does
// not exercise every preset (mirrors the same attribute on `group.rs`/`builder.rs`).
#![allow(dead_code)]

use std::sync::Arc;

use super::super::harness::HostRuntimeCapabilityHarness;
use super::super::harness::options::ToolsProfile;
use super::{
    GroupBaseData, GroupCapability, HarnessResult, RebornIntegrationGroup,
    RebornIntegrationGroupBuilder,
};

/// Shared "align user to the group's canonical binding subject, then build"
/// step for the preset constructors below whose capability executes under
/// the group's resolved binding user rather than a fixed constructor test
/// user (`live_approvals`, `live_auth_and_approval`, `profile_tools`,
/// `outbound_target_tools`). Does NOT cover `skill_activation_tools`
/// (alignment is a constructor-time tenant param plus a post-build skill
/// seed) or `multiuser_approvals` (alignment is
/// `.with_run_owner_scoped_capability_dispatch()`, not a fixed `user_id`
/// override) — those remain call-site-specific.
async fn build_group_capability_with_base(
    mut profile: ToolsProfile,
    base: &GroupBaseData,
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    let subject_user = base.canonical_subject_user()?;
    let product_scope = &base.product_harness.scope;
    let agent_id = product_scope
        .agent_id
        .clone()
        .ok_or("group product scope is missing an agent id")?;
    profile.options = profile
        .options
        .with_local_runtime_identity(product_scope.tenant_id.clone(), agent_id);
    let harness = profile.build().await?;
    Ok(harness.with_user_id(subject_user))
}

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
    /// (extension_search/install/remove). Auto-approve is enabled;
    /// registry credentials are seeded.
    pub async fn extension_lifecycle() -> HarnessResult<Self> {
        Self::builder().extension_lifecycle().await
    }

    /// Extension-lifecycle group extended with the invented-vendor fixture
    /// (native factory + on-disk assets): drives the full generic runtime
    /// path — install → dispatch-from-snapshot → remove — with
    /// no real product (extension-runtime P2).
    pub async fn extension_runtime_acme() -> HarnessResult<Self> {
        Self::builder().extension_runtime_acme().await
    }

    /// Acme runtime group extended for the §5.4 delivery proofs: the bundled
    /// telegram package's native channel factory is assembled and the
    /// recording network egress answers vendor-shaped Slack/Telegram bodies,
    /// so outbound deliveries drive the REAL coordinator → adapter → wire
    /// path (extension-runtime P5, DEL-10).
    pub async fn extension_delivery() -> HarnessResult<Self> {
        Self::builder().extension_delivery().await
    }

    /// Same group as [`Self::extension_lifecycle`], with a Google OAuth
    /// backend configured at composition time. Proves the
    /// provider-instance readiness check does not false-positive once an
    /// operator has run `config set` + restarted — activation falls through
    /// to the ordinary per-account credential gate.
    pub async fn extension_lifecycle_google_oauth_configured() -> HarnessResult<Self> {
        Self::builder()
            .extension_lifecycle_google_oauth_configured()
            .await
    }

    /// Group with the two-capability visibility-probe fixture published into
    /// the active registry and BOTH capabilities granted, so tests can pin
    /// that only the manifest `visibility` value keeps the `host_internal`
    /// sibling off the model surface.
    pub async fn extension_visibility_probe() -> HarnessResult<Self> {
        Self::builder().extension_visibility_probe().await
    }

    /// Group whose GitHub extension's credential account resolves to
    /// `AuthRequired`, so a scripted `github.*` tool call raises a real
    /// `TurnStatus::BlockedAuth` gate (E-AUTHGATE seam). Drive with
    /// `submit_turn_until_auth_blocked`.
    pub async fn live_auth_gate() -> HarnessResult<Self> {
        Self::builder().live_auth_gate().await
    }

    /// C-JOURNEY: surfaces BOTH an unseeded GitHub capability (`BlockedAuth`,
    /// resolve via `resolve_auth_gate`/`deny_auth_gate`) AND real file-tool
    /// approvals (`BlockedApproval`, via `approve_gate`/`deny_gate`) on ONE
    /// `build_reborn_services` runtime. Unlike `live_auth_gate` (a hardcoded
    /// credential resolver, no run_state store), the auth gate here resolves
    /// through the REAL `ProductAuthRuntimeCredentialResolver`, so
    /// `resolve_auth_gate` actually completes. Auto-approve disabled at
    /// construction.
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

    /// Trigger verbs plus `builtin.write_file` on one runtime (#5886
    /// blocked-trigger visibility). Auto-approve is ON so the verbs dispatch
    /// gate-free; a scenario gates the write via
    /// `set_ask_each_time_override_for_test`.
    pub async fn triggers_with_gated_write() -> HarnessResult<Self> {
        Self::builder().triggers_with_gated_write().await
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

    /// Group with the five `builtin.trace_commons.*` capabilities (enabler
    /// (c): the C-TRACECAP enrollment surface). Reuses the SAME
    /// `profiles::trace_commons::trace_commons_tools()` preset the QA/trace
    /// tier uses, over the real turn -> capability path. Auto-approve is
    /// enabled by the profile so scripted onboard/status calls are not gated.
    pub async fn trace_commons_tools() -> HarnessResult<Self> {
        Self::builder().trace_commons_tools().await
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
        // Align capability execution to the run's CANONICAL binding subject user
        // (not the constructor's fixed test user) so dispatch, approval, auto-
        // approve keying, and gate-evidence lookup share one `(tenant, user)` —
        // matching production. `build_group_capability_with_base` (above) is the
        // shared "build then align user" core.
        let host_runtime = build_group_capability_with_base(
            super::super::harness::profiles::file::file_tools_requiring_approval_profile()?,
            &base,
        )
        .await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        let group = self.into_group(base, capability).await?;
        // Disable auto-approve once so every thread faces real approval gates;
        // always `HostRuntime` here, so `Some` is guaranteed.
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
        let host_runtime =
            super::super::harness::profiles::core_builtin::core_builtin_tools_default().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build an extension-lifecycle group. See [`RebornIntegrationGroup::extension_lifecycle`].
    pub async fn extension_lifecycle(self) -> HarnessResult<RebornIntegrationGroup> {
        self.extension_lifecycle_with_profile(
            super::super::harness::profiles::extension::extension_lifecycle_tools_profile_for_user,
        )
        .await
    }

    /// Same group as [`Self::extension_lifecycle`], but with a Google OAuth
    /// backend configured at composition time (the "config set" + restart
    /// arm of the provider-instance readiness map). A SEPARATE composition
    /// build rather than a toggle on the shared `extension_lifecycle()`
    /// group — a real `config
    /// set` + service restart is a new process, not a live flip, so a second
    /// `#[tokio::test]`-local build is the honest analog. See
    /// [`RebornIntegrationGroup::extension_lifecycle_google_oauth_configured`].
    pub async fn extension_lifecycle_google_oauth_configured(
        self,
    ) -> HarnessResult<RebornIntegrationGroup> {
        self.extension_lifecycle_with_profile(
            super::super::harness::profiles::extension::extension_lifecycle_tools_profile_google_oauth_configured_for_user,
        )
        .await
    }

    /// Shared assembly for [`Self::extension_lifecycle`] and
    /// [`Self::extension_lifecycle_google_oauth_configured`] — identical
    /// except for which user-scoped `ToolsProfile` factory the caller selects
    /// (with or without the Google OAuth backend option).
    async fn extension_lifecycle_with_profile(
        mut self,
        profile_for_user: fn(&str) -> HarnessResult<ToolsProfile>,
    ) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        // Lifecycle ownership is caller-derived. Build the profile with the
        // canonical binding subject before credentials are seeded, then align
        // the shared capability harness to that same subject. Building first
        // with the fixed fixture user and only calling `with_user_id` would
        // leave the credential rows under the old user and incorrectly block
        // otherwise credential-ready installs on auth.
        let subject_user = base.canonical_subject_user()?;
        let profile = profile_for_user(subject_user.as_str())?;
        let host_runtime = build_group_capability_with_base(profile, &base).await?;
        // C-SLACK-LIFECYCLE (issue #6105): wire the REAL generic
        // channel-connection facade over this harness's own `RebornServices`,
        // mirroring the production `build_reborn_runtime` slot fill — so
        // `builtin.extension_remove` of a channel extension runs the real
        // per-caller disconnect instead of skipping it on an empty facade
        // slot. Identities come from the group's single-source dispatch scope
        // so the facade's tenant check matches dispatch-time callers.
        let scope = &base.product_harness.scope;
        let channel_connection =
            ironclaw_reborn_composition::test_support::build_channel_connection_for_test(
                host_runtime
                    .reborn_services_for_test()
                    .ok_or("extension_lifecycle harness is missing its RebornServices bundle")?,
                ironclaw_reborn_composition::test_support::ChannelConnectionTestConfig {
                    tenant_id: scope.tenant_id.as_str().to_string(),
                    agent_id: scope
                        .agent_id
                        .as_ref()
                        .map(|agent| agent.as_str().to_string())
                        .ok_or("group product scope is missing an agent id")?,
                },
            )?;
        self.channel_connection = Some(Arc::new(channel_connection));
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build the invented-vendor fixture group. See
    /// [`RebornIntegrationGroup::extension_runtime_acme`].
    pub async fn extension_runtime_acme(mut self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        let host_runtime =
            super::super::harness::profiles::extension::extension_runtime_acme_tools().await?;
        // Same slot fill as `extension_lifecycle` above: acme-messenger
        // declares a channel surface backed by an auth vendor, so
        // `builtin.extension_remove` fail-closes on an empty channel
        // disconnect slot once removal runs under an authenticated actor.
        // Wire the real generic facade over this harness's own
        // `RebornServices`, keyed to the group's dispatch scope.
        let scope = &base.product_harness.scope;
        let channel_connection =
            ironclaw_reborn_composition::test_support::build_channel_connection_for_test(
                host_runtime
                    .reborn_services_for_test()
                    .ok_or("extension_runtime_acme harness is missing its RebornServices bundle")?,
                ironclaw_reborn_composition::test_support::ChannelConnectionTestConfig {
                    tenant_id: scope.tenant_id.as_str().to_string(),
                    agent_id: scope
                        .agent_id
                        .as_ref()
                        .map(|agent| agent.as_str().to_string())
                        .ok_or("group product scope is missing an agent id")?,
                },
            )?;
        self.channel_connection = Some(Arc::new(channel_connection));
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build a delivery-proof group. See
    /// [`RebornIntegrationGroup::extension_delivery`].
    pub async fn extension_delivery(mut self) -> HarnessResult<RebornIntegrationGroup> {
        self.run_delivery_events = Some(Arc::new(
            ironclaw_product::RunDeliveryEventRouter::new_ephemeral_for_test(),
        ));
        let base = self.build_base().await?;
        let host_runtime = build_group_capability_with_base(
            super::super::harness::profiles::extension::extension_delivery_tools_profile()?,
            &base,
        )
        .await?
        .with_run_owner_scoped_capability_dispatch();
        let scope = &base.product_harness.scope;
        let channel_connection =
            ironclaw_reborn_composition::test_support::build_channel_connection_for_test(
                host_runtime
                    .reborn_services_for_test()
                    .ok_or("extension_delivery harness is missing its RebornServices bundle")?,
                ironclaw_reborn_composition::test_support::ChannelConnectionTestConfig {
                    tenant_id: scope.tenant_id.as_str().to_string(),
                    agent_id: scope
                        .agent_id
                        .as_ref()
                        .map(|agent| agent.as_str().to_string())
                        .ok_or("group product scope is missing an agent id")?,
                },
            )?;
        self.channel_connection = Some(Arc::new(channel_connection));
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build a visibility-probe group. See
    /// [`RebornIntegrationGroup::extension_visibility_probe`].
    pub async fn extension_visibility_probe(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime =
            super::super::harness::profiles::extension::extension_visibility_probe_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build an auth-gate group. See [`RebornIntegrationGroup::live_auth_gate`].
    ///
    /// No auto-approve disable and no approval-gate evidence: auth gates are
    /// self-evidencing via the BeforeBlock checkpoint (loop_exit_applier.rs). Do
    /// NOT add approval-gate evidence here — that store is only for approval gates.
    pub async fn live_auth_gate(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime =
            super::super::harness::profiles::github::github_issue_tools_auth_required().await?;
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
        // `build_group_capability_with_base` (above) is the shared "build then
        // align user" core — see `live_approvals` above.
        let host_runtime = build_group_capability_with_base(
            super::super::harness::profiles::github::file_and_github_auth_tools_profile()?,
            &base,
        )
        .await?;
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
        let host_runtime = super::super::harness::profiles::project::project_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a project-lifecycle group with the `project_create`
    /// fault-injection arm wired. See
    /// [`RebornIntegrationGroup::project_lifecycle_fault_injected`].
    pub async fn project_lifecycle_fault_injected(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime =
            super::super::harness::profiles::project::project_tools_with_fault_injection().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a profile-tools group. See [`RebornIntegrationGroup::profile_tools`].
    pub async fn profile_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        // Align `builtin.profile_set`'s executor to the canonical subject user
        // (mirrors `live_approvals`) — otherwise a write and its read-back
        // resolve under different users. Needs `base` first, so can't go
        // through `build_with_capability`.
        let host_runtime = build_group_capability_with_base(
            super::super::harness::profiles::profile::profile_tools_profile()?,
            &base,
        )
        .await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build a trigger-management group. See [`RebornIntegrationGroup::triggers`].
    pub async fn triggers(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime =
            super::super::harness::profiles::trigger::trigger_management_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a triggers-plus-gated-write group. See
    /// [`RebornIntegrationGroup::triggers_with_gated_write`].
    pub async fn triggers_with_gated_write(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime =
            super::super::harness::profiles::trigger::trigger_management_with_gated_write().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a skill-activation group. See
    /// [`RebornIntegrationGroup::skill_activation_tools`]. The skill profile
    /// pre-seeds the system fixtures before runtime construction so the warmed
    /// system-skill descriptor cache sees them.
    pub async fn skill_activation_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        // Pass the group's ACTUAL run-scope tenant (resolved by `build_base`
        // above) rather than a separately hardcoded literal, so the E-SKILL
        // skill context source is built for the same tenant the turn runs
        // under — see `HostRuntimeCapabilityHarness::skill_activation_tools`.
        let host_runtime = super::super::harness::profiles::skill::skill_activation_tools(
            &base.canonical_binding.tenant_id,
        )
        .await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Per-actor-scoped memory group. See
    /// [`RebornIntegrationGroup::multiuser_memory_tools`]. Same capability
    /// surface as [`builtin_tools`] but with per-actor capability dispatch, so
    /// each actor's memory lands under its own owner subtree.
    pub async fn multiuser_memory_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime =
            super::super::harness::profiles::core_builtin::core_builtin_tools_default()
                .await?
                .with_run_owner_scoped_capability_dispatch();
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Per-actor-scoped file-approval group. See
    /// [`RebornIntegrationGroup::multiuser_approvals`]. Real approval stores
    /// (write_file/read_file @ `Ask`) plus per-actor capability dispatch;
    /// auto-approve defaults ON per owner, so a test that needs an owner to
    /// GATE sets that owner OFF via `disable_auto_approve_for_owner` — the
    /// per-user setting is what isolation asserts. Dispatch user == turn owner,
    /// so the raised approval's gate-evidence lookup resolves under that same
    /// owner (verified, not masked to `Failed`).
    pub async fn multiuser_approvals(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        let host_runtime = super::super::harness::profiles::file::file_tools_requiring_approval()
            .await?
            .with_run_owner_scoped_capability_dispatch();
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Outbound-target-tools group. See
    /// [`RebornIntegrationGroup::outbound_target_tools`]. Mirrors
    /// `live_approvals`/`profile_tools`: the synthetic `outbound_delivery_*`
    /// capabilities run under the run's canonical binding subject user, so the
    /// dispatch-time auto-approve scope aligns with tests' per-test disables.
    /// Auto-approve stays default-ON here; the gate arm disables it per-test.
    pub async fn outbound_target_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        // `build_group_capability_with_base` (above) is the shared "build then
        // align user" core — see `live_approvals` above.
        let host_runtime = build_group_capability_with_base(
            super::super::harness::profiles::outbound::outbound_target_tools_profile()?,
            &base,
        )
        .await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.into_group(base, capability).await
    }

    /// Build a skill-management group. See
    /// [`RebornIntegrationGroup::skill_management_tools`].
    pub async fn skill_management_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = super::super::harness::profiles::skill::skill_management_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build a trace-commons group. See
    /// [`RebornIntegrationGroup::trace_commons_tools`].
    pub async fn trace_commons_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime =
            super::super::harness::profiles::trace_commons::trace_commons_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }

    /// Build an attachment-tools group. See [`RebornIntegrationGroup::attachment_tools`].
    pub async fn attachment_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let host_runtime = super::super::harness::profiles::attachment::attachment_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        self.build_with_capability(capability).await
    }
}
