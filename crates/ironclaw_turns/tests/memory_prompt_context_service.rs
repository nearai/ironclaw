//! Contract tests for [`MemoryPromptContextService`] and [`EmptyMemoryPromptContextService`].

use ironclaw_host_api::{TenantId, ThreadId, UserId};
use ironclaw_turns::run_profile::{
    ContextProfileId, EmptyMemoryPromptContextService, MemoryPromptContextRequest,
    MemoryPromptContextService,
};
use ironclaw_turns::scope::{TurnActor, TurnScope};

fn test_scope(tenant: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new(tenant).unwrap(),
        None,
        None,
        ThreadId::new("thread-1").unwrap(),
    )
}

fn test_actor(user: &str) -> TurnActor {
    TurnActor::new(UserId::new(user).unwrap())
}

fn test_request(tenant: &str, user: &str) -> MemoryPromptContextRequest {
    MemoryPromptContextRequest {
        scope: test_scope(tenant),
        actor: test_actor(user),
        query: "test query".to_string(),
        max_snippets: 10,
        context_profile_id: ContextProfileId::new("default").unwrap(),
    }
}

#[tokio::test]
async fn empty_service_returns_empty_vec() {
    let service = EmptyMemoryPromptContextService;
    let result = service
        .load_memory_snippets(test_request("tenant-a", "user-x"))
        .await;
    let snippets = result.expect("empty service should not fail");
    assert!(snippets.is_empty());
}

#[tokio::test]
async fn request_carries_scope_and_actor_fields() {
    let request = test_request("tenant-abc", "user-xyz");
    assert_eq!(request.scope.tenant_id.as_str(), "tenant-abc");
    assert_eq!(request.actor.user_id.as_str(), "user-xyz");
    assert_eq!(request.query, "test query");
    assert_eq!(request.max_snippets, 10);
}
