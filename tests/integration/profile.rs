//! E-PROFILE seam smoke test: `profile_tools()` wires ONE planned runtime with
//! a real `HostUserProfileSource` (backed by the local-dev memory filesystem
//! `builtin.profile_set` writes through) instead of `EmptyUserProfileSource`.
//!
//! A scripted `builtin.profile_set` call dispatches through the real
//! production capability; the test reads back through the SAME
//! `Arc<dyn HostUserProfileSource>` instance the group's runtime uses
//! (`user_profile_source_for_test`), so a regression in `into_group` wiring
//! itself — not just `build_user_profile_source_for_test` — fails this test.
//!
//! Mutation-catching: an ignored filesystem input or an always-`None` source
//! makes `resolve_user_profile` return `None`, failing the `expect` below.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::group::IronClawIntegrationGroup;
use ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::run_profile::{
    InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest,
};
use ironclaw_turns::{RunProfileResolver, TurnActor, TurnId, TurnRunId, TurnScope};

/// Build the `LoopRunContext` `resolve_user_profile` reads from, scoped to the
/// same `(tenant, user)` the `profile_set` write dispatched under — the
/// capability harness's dispatch user, which `profile_tools()` aligns to the
/// binding subject user via `with_user_id` (mirroring `live_approvals`; see
/// `HostRuntimeCapabilityHarness::with_user_id`).
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
    let group = IronClawIntegrationGroup::profile_tools()
        .await
        .expect("profile-tools group builds");
    let harness = group
        .thread("conv-profile-set")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.profile_set",
                serde_json::json!({
                    "timezone": "America/Los_Angeles",
                    "locale": "en-US",
                    "location": "Los Angeles, USA",
                }),
            ),
            IronClawScriptedReply::text("saved your profile"),
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

    // Read back through the SAME HostUserProfileSource (E-PROFILE seam), keyed
    // by the dispatch tenant/user (see `read_back_run_context` above).
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

    // Profile resolves once per loop spawn, before turn 1's write — a second
    // thread's fresh loop is needed to observe it in the model-visible prompt.
    let prompt_thread = group
        .thread("conv-profile-prompt")
        .script([IronClawScriptedReply::text("ok")])
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
