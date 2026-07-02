//! E-PROFILE seam smoke test: `profile_tools()` builds a `RebornIntegrationGroup`
//! whose ONE planned runtime is wired with a real `HostUserProfileSource`
//! (`build_user_profile_source_for_test`, backed by the local-dev memory
//! filesystem `builtin.profile_set` writes through) instead of the default
//! `EmptyUserProfileSource`.
//!
//! A scripted `builtin.profile_set` tool call dispatches through the REAL
//! production capability (`crates/ironclaw_host_runtime/src/first_party_tools/profile_set.rs`),
//! and the test then reads the profile back through the SAME
//! `Arc<dyn HostUserProfileSource>` instance the group's planned runtime is
//! built with (`RebornIntegrationGroup::user_profile_source_for_test`) — not a
//! re-derived equivalent — so a regression in the `into_group` wiring itself
//! (not just `build_user_profile_source_for_test`) fails this test.
//!
//! Mutation-catching: if the profile source ignores its filesystem input (or
//! always returns `EmptyUserProfileSource`/`None`), `resolve_user_profile`
//! returns `None` here and the `expect` below fails.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use ironclaw_turns::run_profile::{
    InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest,
};
use ironclaw_turns::{RunProfileResolver, TurnActor, TurnId, TurnRunId, TurnScope};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

/// Build the `LoopRunContext` `resolve_user_profile` reads from, scoped to the
/// same `(tenant, user)` the `profile_set` write dispatched under: the
/// thread's binding tenant and the capability harness's dispatch user, which
/// `RebornIntegrationGroup::profile_tools()` now aligns to the run's
/// canonical binding subject user via `with_user_id` (mirroring
/// `live_approvals`) — see `HostRuntimeCapabilityHarness::with_user_id` doc
/// comment — dispatch's `ResourceScope` is keyed on the harness user, not the
/// binding owner, unless overridden.
async fn read_back_run_context(tenant_id: &str, user_id: &str) -> LoopRunContext {
    let resolved_run_profile = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("resolve interactive run profile");
    let scope = TurnScope::new(
        ironclaw_host_api::TenantId::new(tenant_id).expect("valid tenant id"),
        None,
        None,
        ironclaw_host_api::ThreadId::new("thread-profile-itest").expect("valid thread id"),
    );
    let actor = TurnActor::new(ironclaw_host_api::UserId::new(user_id).expect("valid user id"));
    LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
        .with_actor(actor)
}

#[tokio::test]
async fn profile_set_write_is_readable_through_the_wired_profile_source() {
    let group = RebornIntegrationGroup::profile_tools()
        .await
        .expect("profile-tools group builds");
    let harness = group
        .thread("conv-profile-set")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.profile_set",
                serde_json::json!({
                    "timezone": "America/Los_Angeles",
                    "locale": "en-US",
                    "location": "Los Angeles, USA",
                }),
            ),
            RebornScriptedReply::text("saved your profile"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("remember I'm in Los Angeles")
        .await
        .expect("turn completes");

    harness
        .assert_tool_invoked("builtin.profile_set")
        .await
        .expect("profile_set dispatched through the real capability");

    // Read back through the SAME `HostUserProfileSource` the group's planned
    // runtime was built with (E-PROFILE seam), keyed by the dispatch tenant
    // (the thread's binding tenant) and the capability harness's dispatch
    // user (now the run's aligned canonical binding subject user, per the
    // `profile_tools()` alignment fix in `group.rs`).
    let dispatch_user = group
        .capability_harness()
        .expect("profile_tools always uses HostRuntime")
        .user_id()
        .as_str()
        .to_string();
    let run_context =
        read_back_run_context(harness.binding.tenant_id.as_str(), &dispatch_user).await;
    let resolved = group
        .user_profile_source_for_test()
        .resolve_user_profile(&run_context)
        .await
        .expect("profile_set write must be readable through the wired HostUserProfileSource");

    assert_eq!(
        resolved.timezone.map(|tz| tz.name().to_string()),
        Some("America/Los_Angeles".to_string()),
        "timezone must survive the profile_set -> HostUserProfileSource round trip"
    );
    assert_eq!(
        resolved.locale.as_ref().map(|l| l.as_str()),
        Some("en-US"),
        "locale must survive the round trip"
    );
    assert_eq!(
        resolved.location.as_deref(),
        Some("Los Angeles, USA"),
        "location must survive the round trip"
    );

    // The profile is resolved once per loop spawn (before turn 1's profile_set write), so a
    // SECOND loop is needed to observe it in the model-visible prompt. A second thread in the
    // same group now shares the profile source under the SAME aligned (tenant, user) as turn 1
    // (group.rs profile_tools() alignment fix), so its fresh loop renders the now-written
    // profile into the runtime-context system message.
    let prompt_thread = group
        .thread("conv-profile-prompt")
        .script([RebornScriptedReply::text("ok")])
        .build()
        .await
        .expect("prompt thread builds");
    prompt_thread
        .submit_turn("what's my setup")
        .await
        .expect("turn completes");
    prompt_thread
        .assert_system_prompt_contains("locale=en-US")
        .await
        .expect("profile_set locale must reach the model-visible system prompt");
    prompt_thread
        .assert_system_prompt_contains("America/Los_Angeles")
        .await
        .expect("profile_set timezone must reach the model-visible system prompt");
}
