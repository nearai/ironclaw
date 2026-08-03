//! Thread-scope validation tests for `loop_driver_host::validate_thread_scope`.
//!
//! Split out of `loop_driver_host/tests.rs` when the port adapters that file
//! also covered moved to `ironclaw_loop_host` (WS3 runner sheds). The three
//! tests below are unchanged; only the module they live in moved.

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_contracts::{
    InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest, RunProfileResolver,
};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::{TurnActor, TurnId, TurnRunId, TurnScope};

async fn test_run_context() -> LoopRunContext {
    let tenant_id = TenantId::new("tenant-surf-prompt-test").unwrap();
    let agent_id = AgentId::new("agent-surf-prompt-test").unwrap();
    let project_id = ProjectId::new("project-surf-prompt-test").unwrap();
    let thread_id = ThreadId::new("thread-surf-prompt-test").unwrap();
    let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved)
}

fn thread_scope_for(context: &LoopRunContext, owner: Option<UserId>) -> ThreadScope {
    ThreadScope {
        tenant_id: context.scope.tenant_id.clone(),
        agent_id: context
            .scope
            .agent_id
            .clone()
            .expect("test run context is agent-scoped"),
        project_id: context.scope.project_id.clone(),
        owner_user_id: owner,
        mission_id: None,
    }
}

#[tokio::test]
async fn validate_thread_scope_rejects_owner_mismatch() {
    // Defense in depth for the thread-owner MountView divergence: the thread
    // store keys threads by owner, so a host thread scope whose owner differs
    // from the run's authenticated actor silently reads the wrong
    // `owners/<user>` subtree and fails with `UnknownThread`. Fail loud here
    // instead.
    let context = test_run_context()
        .await
        .with_actor(TurnActor::new(UserId::new("local-user").unwrap()));
    let thread_scope = thread_scope_for(&context, Some(UserId::new("reborn-cli").unwrap()));

    let error = super::validate_thread_scope(&thread_scope, &context)
        .expect_err("owner mismatch must be rejected");
    assert!(matches!(
        error,
        super::RebornLoopDriverHostError::ScopeMismatch { .. }
    ));
}

#[tokio::test]
async fn validate_thread_scope_accepts_matching_owner() {
    let context = test_run_context()
        .await
        .with_actor(TurnActor::new(UserId::new("local-user").unwrap()));
    let thread_scope = thread_scope_for(&context, Some(UserId::new("local-user").unwrap()));

    super::validate_thread_scope(&thread_scope, &context).expect("matching owner must validate");
}

#[tokio::test]
async fn validate_thread_scope_skips_owner_check_without_actor() {
    // When the run carries no actor (system/legacy turns), the owner axis
    // cannot be cross-checked; the guard must not reject these.
    let context = test_run_context().await;
    let thread_scope = thread_scope_for(&context, Some(UserId::new("local-user").unwrap()));

    super::validate_thread_scope(&thread_scope, &context)
        .expect("absent actor must skip the owner check");
}
