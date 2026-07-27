//! Provider-level contract suite for [`MemoryService`](crate::MemoryService)
//! implementations.
//!
//! Same scaffolding pattern as `ironclaw_memory_native::contract_tests`
//! (factory closures, one `#[tokio::test]` per contract per impl via a wiring
//! macro): the invariants every bound memory provider must honor are defined
//! ONCE here, and each provider crate wires the suite over a real (or
//! stateful-fake) backing so the contract is proven per impl, not per mock.
//!
//! Contracts:
//! - **Scope isolation** across tenant/user/agent/project: content written in
//!   one scope is invisible to retrieval in any scope differing on any axis.
//! - **Lane disjointness** (F4 regression) for providers declaring BOTH
//!   retrieval lanes: a short-term (active-thread) result never appears in
//!   the long-term lane and vice versa, so the host's concatenation can never
//!   double-spend its byte budget on duplicates.
//! - **`record_interaction` round-trip visibility** (F5 regression) for
//!   providers declaring the hook: a recorded exchange reports
//!   `recorded: true` and is retrievable from the short-term lane.
//!
//! F6 note (owner decision, 2026-07-27): there is deliberately NO profile-CAS
//! contract — profile-write atomicity is per-provider (native CASes; mem0
//! read-merge-writes), accepted variance behind the shared tool id.
//!
//! Tool surfaces are provider-owned (registry-routed handlers), so the suite
//! cannot seed content through a shared trait method: each wiring supplies a
//! SEED async closure that writes through the provider's own write operation.
//!
//! Wire with [`memory_service_contract_full!`] (both lanes + recording — the
//! native shape) or [`memory_service_contract_retrieval_only!`] (a
//! long-term-only provider — the mem0 shape); both take
//! `(label, factory, seed)` where `seed` is
//! `async |service, invocation, write_request| { … }`.

use ironclaw_host_api::{
    AgentId, CorrelationId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};

use crate::{
    MemoryContextProfileId, MemoryInteractionMessage, MemoryInteractionRole, MemoryInvocation,
    MemoryService, MemoryServiceContextRequest, MemoryServiceContextSnippet,
    MemoryServiceRecordRequest, MemoryServiceWriteRequest,
};

/// Base scope every contract writes under: the full four-axis tuple, so each
/// probe scope can differ on exactly one axis.
fn base_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-contract").expect("tenant id"),
        user_id: UserId::new("user-contract").expect("user id"),
        agent_id: Some(AgentId::new("agent-contract").expect("agent id")),
        project_id: Some(ProjectId::new("project-contract").expect("project id")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn invocation(scope: ResourceScope) -> MemoryInvocation {
    MemoryInvocation {
        scope,
        correlation_id: CorrelationId::new(),
    }
}

fn with_thread(mut scope: ResourceScope, thread: &str) -> ResourceScope {
    scope.thread_id = Some(ThreadId::new(thread).expect("thread id"));
    scope
}

fn context_request(query: &str) -> MemoryServiceContextRequest {
    MemoryServiceContextRequest {
        query: query.to_string(),
        max_snippets: 10,
        context_profile_id: MemoryContextProfileId::new("default").expect("profile id"),
    }
}

fn write_request(target: &str, content: &str) -> MemoryServiceWriteRequest {
    MemoryServiceWriteRequest {
        target: target.to_string(),
        content: content.to_string(),
        append: true,
        old_string: None,
        new_string: None,
        replace_all: false,
        metadata: None,
        timezone: None,
    }
}

fn texts(snippets: &[MemoryServiceContextSnippet]) -> Vec<&str> {
    snippets
        .iter()
        .map(|snippet| snippet.text.as_str())
        .collect()
}

/// Contract: content written in the base scope is retrievable there and
/// invisible to every scope differing on exactly one of
/// tenant/user/agent/project.
pub async fn scope_isolation_across_tenant_user_agent_project<S, F, Seed>(factory: F, seed: Seed)
where
    S: MemoryService,
    F: Fn() -> S,
    Seed: AsyncFn(&S, MemoryInvocation, MemoryServiceWriteRequest),
{
    let service = factory();
    let base = base_scope();
    seed(
        &service,
        invocation(base.clone()),
        write_request("notes/isolation.md", "contract isolation marker kestrel"),
    )
    .await;

    let own = service
        .read_long_term(invocation(base.clone()), context_request("kestrel"))
        .await
        .expect("base-scope retrieval must succeed");
    assert!(
        own.iter().any(|snippet| snippet.text.contains("kestrel")),
        "the writing scope must retrieve its own content (got {:?})",
        texts(&own)
    );

    let mut other_tenant = base.clone();
    other_tenant.tenant_id = TenantId::new("tenant-other").expect("tenant id");
    let mut other_user = base.clone();
    other_user.user_id = UserId::new("user-other").expect("user id");
    let mut other_agent = base.clone();
    other_agent.agent_id = Some(AgentId::new("agent-other").expect("agent id"));
    let mut other_project = base.clone();
    other_project.project_id = Some(ProjectId::new("project-other").expect("project id"));

    for (axis, probe) in [
        ("tenant", other_tenant),
        ("user", other_user),
        ("agent", other_agent),
        ("project", other_project),
    ] {
        let cross = service
            .read_long_term(invocation(probe), context_request("kestrel"))
            .await
            .expect("cross-scope retrieval must succeed (empty, not error)");
        assert!(
            cross
                .iter()
                .all(|snippet| !snippet.text.contains("kestrel")),
            "a scope differing on the {axis} axis must not see the base scope's \
             content (got {:?})",
            texts(&cross)
        );
    }
}

/// Contract (F4): for a provider declaring BOTH retrieval lanes, the lanes
/// stay disjoint under the SAME thread-carrying invocation — a short-term
/// (active-thread) result never appears in the long-term lane, and the
/// short-term lane never surfaces general memory.
pub async fn retrieval_lanes_are_disjoint<S, F, Seed>(factory: F, seed: Seed)
where
    S: MemoryService,
    F: Fn() -> S,
    Seed: AsyncFn(&S, MemoryInvocation, MemoryServiceWriteRequest),
{
    let service = factory();
    let scoped = with_thread(base_scope(), "thread-disjoint");

    seed(
        &service,
        invocation(scoped.clone()),
        write_request("notes/general.md", "osprey durable general fact"),
    )
    .await;
    let recorded = service
        .record_interaction(
            invocation(scoped.clone()),
            MemoryServiceRecordRequest {
                messages: vec![MemoryInteractionMessage {
                    role: MemoryInteractionRole::User,
                    content: "osprey scratch note for this thread".to_string(),
                    name: Some("user-contract".to_string()),
                }],
                turn_run_id: Some("run-disjoint-1".to_string()),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("record_interaction must succeed");
    assert!(recorded.recorded, "the thread exchange must be recorded");

    let long = service
        .read_long_term(invocation(scoped.clone()), context_request("osprey"))
        .await
        .expect("long-term lane must succeed");
    let short = service
        .read_short_term(invocation(scoped), context_request("osprey"))
        .await
        .expect("short-term lane must succeed");

    assert!(
        long.iter().any(|snippet| snippet.text.contains("durable")),
        "long-term lane must surface the general doc (got {:?})",
        texts(&long)
    );
    assert!(
        short.iter().any(|snippet| snippet.text.contains("scratch")),
        "short-term lane must surface the recorded thread doc (got {:?})",
        texts(&short)
    );
    // Disjointness both ways: no snippet path may appear in both lanes, the
    // long-term lane carries no thread scratch, and the short-term lane
    // carries no general memory.
    for snippet in &long {
        assert!(
            !short
                .iter()
                .any(|other| other.relative_path == snippet.relative_path),
            "lanes must be disjoint; {} appeared in both (F4)",
            snippet.relative_path
        );
        assert!(
            !snippet.text.contains("scratch"),
            "long-term lane leaked thread scratch: {:?}",
            snippet.text
        );
    }
    for snippet in &short {
        assert!(
            !snippet.text.contains("durable"),
            "short-term lane leaked general memory: {:?}",
            snippet.text
        );
    }
}

/// Contract (F5): a provider declaring `record_interaction` really records —
/// the response reports `recorded: true` and the exchange is retrievable
/// from the short-term lane of the SAME thread (and only that thread).
pub async fn record_interaction_round_trips_into_retrieval<S, F>(factory: F)
where
    S: MemoryService,
    F: Fn() -> S,
{
    let service = factory();
    let scoped = with_thread(base_scope(), "thread-record");

    let recorded = service
        .record_interaction(
            invocation(scoped.clone()),
            MemoryServiceRecordRequest {
                messages: vec![
                    MemoryInteractionMessage {
                        role: MemoryInteractionRole::User,
                        content: "remember the merlin launch window".to_string(),
                        name: Some("user-contract".to_string()),
                    },
                    MemoryInteractionMessage {
                        role: MemoryInteractionRole::Assistant,
                        content: "noted: merlin launch window".to_string(),
                        name: Some("agent-contract".to_string()),
                    },
                ],
                turn_run_id: Some("run-record-1".to_string()),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("record_interaction must succeed");
    assert!(
        recorded.recorded,
        "a declared recorder must report recorded: true (F5 — a silent no-op \
         behind a declared hook is exactly the defect this pins)"
    );

    let same_thread = service
        .read_short_term(invocation(scoped.clone()), context_request("merlin"))
        .await
        .expect("short-term retrieval must succeed");
    assert!(
        same_thread
            .iter()
            .any(|snippet| snippet.text.contains("merlin")),
        "the recorded exchange must be retrievable from its thread's \
         short-term lane (got {:?})",
        texts(&same_thread)
    );

    let other_thread = service
        .read_short_term(
            invocation(with_thread(base_scope(), "thread-unrelated")),
            context_request("merlin"),
        )
        .await
        .expect("other-thread retrieval must succeed");
    assert!(
        other_thread
            .iter()
            .all(|snippet| !snippet.text.contains("merlin")),
        "another thread's short-term lane must not see the recording (got {:?})",
        texts(&other_thread)
    );
}

/// Wire the FULL provider contract suite (both retrieval lanes + interaction
/// recording — the shape native declares). `$factory` must return a fresh
/// service over a fresh backing per call.
#[macro_export]
macro_rules! memory_service_contract_full {
    ($label:ident, $factory:expr, $seed:expr) => {
        mod $label {
            use super::*;

            #[tokio::test]
            async fn scope_isolation_across_tenant_user_agent_project() {
                $crate::test_support::scope_isolation_across_tenant_user_agent_project(
                    $factory, $seed,
                )
                .await;
            }

            #[tokio::test]
            async fn retrieval_lanes_are_disjoint() {
                $crate::test_support::retrieval_lanes_are_disjoint($factory, $seed).await;
            }

            #[tokio::test]
            async fn record_interaction_round_trips_into_retrieval() {
                $crate::test_support::record_interaction_round_trips_into_retrieval($factory).await;
            }
        }
    };
}

/// Wire the contract suite for a provider declaring ONLY the long-term
/// retrieval lane (the mem0 shape): scope isolation applies; the lane and
/// recording contracts do not (those hooks are undeclared, so the host never
/// calls them).
#[macro_export]
macro_rules! memory_service_contract_retrieval_only {
    ($label:ident, $factory:expr, $seed:expr) => {
        mod $label {
            use super::*;

            #[tokio::test]
            async fn scope_isolation_across_tenant_user_agent_project() {
                $crate::test_support::scope_isolation_across_tenant_user_agent_project(
                    $factory, $seed,
                )
                .await;
            }
        }
    };
}
