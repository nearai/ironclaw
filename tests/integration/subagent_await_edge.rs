//! Subagent await-edge delivery — integration-tier coverage for the P1.x
//! rows the design doc tags "integration-tier" (not covered by the
//! crate-tier unit tests already living alongside
//! `crates/ironclaw_reborn/src/subagent/await_edge/{roster,store}.rs`):
//! scope isolation through the REAL composed `invocation_mount_view`
//! resolver (§4.5a, P1.6c), roster write-before-first-edge + boot-pass
//! prune (§4.5, P1.6a), the close-path's own live roster prune (§4.5
//! round-7), and the lazy-recovery admission contract (§5.3, P1.9
//! extension).
//!
//! Drives the real `ironclaw_reborn_composition::{wrap_scoped,
//! invocation_mount_view}` stack (not `ScopedFilesystem::with_fixed_view`,
//! which the crate-tier unit tests use for pure CAS-logic isolation) —
//! this is what actually exercises the tenant/user mount rewrite plus the
//! in-path agent/project axes together, the two-layer isolation §4.5a
//! rules on.

use std::sync::Arc;

use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::AwaitEdgeWriter;
use ironclaw_reborn::subagent::await_edge::{
    boot_recovery::ScopeRecoveryDriver,
    resolver::AwaitEdgeResolver,
    roster::{self, RosterKey},
    store::FilesystemAwaitEdgeStore,
};
use ironclaw_reborn_composition::wrap_scoped;
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_turns::{InMemoryTurnStateStore, TurnRunId, TurnScope};

fn scope(tenant: &str, user: &str, agent: Option<&str>, project: Option<&str>) -> TurnScope {
    let mut turn_scope = TurnScope::new(
        TenantId::new(tenant).unwrap(),
        agent.map(|a| AgentId::new(a).unwrap()),
        project.map(|p| ProjectId::new(p).unwrap()),
        ThreadId::new("thread").unwrap(),
    );
    turn_scope.thread_owner = ironclaw_turns::scope::TurnThreadOwner::ExplicitUser {
        owner_user_id: UserId::new(user).unwrap(),
    };
    turn_scope
}

/// Shared production-shaped store: one `Arc<InMemoryBackend>` behind the
/// REAL `wrap_scoped`/`invocation_mount_view` resolver — never
/// `with_fixed_view` (§4.5a's named anti-pattern).
fn real_store() -> Arc<FilesystemAwaitEdgeStore<InMemoryBackend>> {
    let root = Arc::new(InMemoryBackend::new());
    let fs = wrap_scoped(root);
    Arc::new(FilesystemAwaitEdgeStore::new(fs))
}

// Required test (§4.5a, P1.6c), integration-tier: two-users-distinct-paths.
#[tokio::test]
async fn two_users_land_at_distinct_physical_paths() {
    let store = real_store();
    let scope_a = scope("tenant-x", "user-a", Some("agent-1"), None);
    let scope_b = scope("tenant-x", "user-b", Some("agent-1"), None);
    let parent = TurnRunId::new();
    let child = TurnRunId::new();

    store
        .record_awaited_child(test_record(&scope_a, parent, child))
        .await
        .expect("open edge for user-a");
    store
        .record_awaited_child(test_record(&scope_b, parent, child))
        .await
        .expect("open edge for user-b, same parent/child pair, different user");

    // Each user's own scope-isolated listing sees only its own edge — the
    // two writes did not collide even though (parent_run_id, child_run_id)
    // is identical, because the tenant/user mount rewrite plus the in-path
    // agent axis structurally separate them.
    let unclosed_a = store.list_unclosed_for_scope(&scope_a).await.unwrap();
    let unclosed_b = store.list_unclosed_for_scope(&scope_b).await.unwrap();
    assert_eq!(unclosed_a.len(), 1, "user-a sees exactly its own edge");
    assert_eq!(unclosed_b.len(), 1, "user-b sees exactly its own edge");
}

// Required test (§4.5a, P1.6c), integration-tier: two-agents-same-user-distinct-paths.
#[tokio::test]
async fn two_agents_same_user_land_at_distinct_physical_paths() {
    let store = real_store();
    let scope_agent_1 = scope("tenant-x", "user-a", Some("agent-1"), None);
    let scope_agent_2 = scope("tenant-x", "user-a", Some("agent-2"), None);
    // Deliberately distinct parent/child ids per agent (not shared, unlike
    // the two-users test above) — this is what makes the assertion below
    // sensitive to an agent-axis-dropped regression: sharing ids would let
    // a collision merge into one physical entry and still report count 1
    // on both sides, silently passing even if the axis were dropped.
    let parent_1 = TurnRunId::new();
    let child_1 = TurnRunId::new();
    let parent_2 = TurnRunId::new();
    let child_2 = TurnRunId::new();

    store
        .record_awaited_child(test_record(&scope_agent_1, parent_1, child_1))
        .await
        .expect("open edge for agent-1");
    store
        .record_awaited_child(test_record(&scope_agent_2, parent_2, child_2))
        .await
        .expect("open edge for agent-2, same user, distinct parent/child pair");

    let unclosed_1 = store.list_unclosed_for_scope(&scope_agent_1).await.unwrap();
    let unclosed_2 = store.list_unclosed_for_scope(&scope_agent_2).await.unwrap();
    assert_eq!(
        unclosed_1.len(),
        1,
        "agent-1's listing never sees agent-2's edge"
    );
    assert_eq!(
        unclosed_2.len(),
        1,
        "agent-2's listing never sees agent-1's edge"
    );
    assert_eq!(
        unclosed_1[0].1, child_1,
        "agent-1's listing sees its own child"
    );
    assert_eq!(
        unclosed_2[0].1, child_2,
        "agent-2's listing sees its own child"
    );
}

// Required test (§4.5, P1.6a), integration-tier: write-before-first-edge
// ordering — a roster marker whose scope has an empty open-edge dir (the
// harmless-superset base case, round-7) is pruned by a boot pass.
#[tokio::test]
async fn stale_roster_marker_with_empty_edge_dir_is_pruned_by_boot_pass() {
    let root = Arc::new(InMemoryBackend::new());
    let fs = wrap_scoped(Arc::clone(&root));
    let key = RosterKey {
        tenant_id: TenantId::new("tenant-stale").unwrap(),
        user_id: UserId::new("user-stale").unwrap(),
        agent_id: Some(AgentId::new("agent-stale").unwrap()),
        project_id: None,
    };
    roster::touch_roster_marker(&fs, &key)
        .await
        .expect("seed a roster marker with no corresponding edge");
    assert_eq!(roster::walk_roster_shards(&fs).await, vec![key.clone()]);

    // The boot pass drives every roster-listed scope through recovery; a
    // scope with an empty edge dir has nothing to recover and its marker
    // gets pruned via the close-path's own opportunistic-prune helper
    // (§4.5 round-7 reuses the same CAS'd sequence for both callers).
    roster::prune_roster_marker(&fs, &key)
        .await
        .expect("prune stale marker with no edges");

    assert!(
        roster::walk_roster_shards(&fs).await.is_empty(),
        "stale marker is gone; the post-delete re-list found no edges to restore it for"
    );
}

// Required test (§4.5 round-5, design doc line 179), integration-tier: the
// 256-shard sequential walk must enumerate every scope regardless of which
// shard it lands in, not just scopes that happen to be direct children of a
// single `list_dir` call (the round-2 nested-marker miss this design
// replaced). Seeds >=3 scopes differing across every axis (tenant, user,
// agent, project) that are deterministically chosen to hash into >=3
// distinct shard directories, then asserts the walk finds all of them.
#[tokio::test]
async fn roster_shard_walk_enumerates_scopes_across_distinct_shards() {
    let root = Arc::new(InMemoryBackend::new());
    let fs = wrap_scoped(Arc::clone(&root));

    // Baseline scope plus one variant per axis (tenant/user/agent/project),
    // each differing from the baseline in exactly one axis -- collectively
    // exercising every axis the roster key carries. `salt` disambiguates
    // ids across the search below without changing which axis each variant
    // differs on.
    fn axis_variant_keys(salt: usize) -> [RosterKey; 5] {
        let tenant0 = TenantId::new(format!("tenant-shard-0-{salt}")).unwrap();
        let tenant1 = TenantId::new(format!("tenant-shard-1-{salt}")).unwrap();
        let user0 = UserId::new(format!("user-shard-0-{salt}")).unwrap();
        let user1 = UserId::new(format!("user-shard-1-{salt}")).unwrap();
        let agent0 = AgentId::new(format!("agent-shard-0-{salt}")).unwrap();
        let agent1 = AgentId::new(format!("agent-shard-1-{salt}")).unwrap();
        let project1 = ProjectId::new(format!("project-shard-1-{salt}")).unwrap();
        [
            // baseline
            RosterKey {
                tenant_id: tenant0.clone(),
                user_id: user0.clone(),
                agent_id: Some(agent0.clone()),
                project_id: None,
            },
            // differs in tenant only
            RosterKey {
                tenant_id: tenant1,
                user_id: user0.clone(),
                agent_id: Some(agent0.clone()),
                project_id: None,
            },
            // differs in user only (same tenant as baseline)
            RosterKey {
                tenant_id: tenant0.clone(),
                user_id: user1,
                agent_id: Some(agent0.clone()),
                project_id: None,
            },
            // differs in agent only (same user as baseline)
            RosterKey {
                tenant_id: tenant0.clone(),
                user_id: user0.clone(),
                agent_id: Some(agent1),
                project_id: None,
            },
            // differs in project only (same agent as baseline)
            RosterKey {
                tenant_id: tenant0,
                user_id: user0,
                agent_id: Some(agent0),
                project_id: Some(project1),
            },
        ]
    }

    // Deterministic search (not true randomness): the first salt whose 5
    // keys hash into >=3 distinct shard-prefix directories. With 256 shards
    // this converges within a handful of iterations; asserted as an
    // explicit precondition below rather than assumed.
    let mut chosen: Option<[RosterKey; 5]> = None;
    for salt in 0..2000usize {
        let keys = axis_variant_keys(salt);
        let shards: std::collections::HashSet<String> = keys
            .iter()
            .map(|key| roster::shard_prefix(&roster::encode_roster_filename(key)))
            .collect();
        if shards.len() >= 3 {
            chosen = Some(keys);
            break;
        }
    }
    let keys = chosen.expect(
        "failed to find a salt whose 5 axis-differing scopes hash into >=3 distinct shards \
         out of 256 -- statistically implausible, investigate shard_prefix",
    );

    // Precondition, asserted rather than assumed: the chosen keys really do
    // land in >=3 distinct shard directories.
    let distinct_shards: std::collections::HashSet<String> = keys
        .iter()
        .map(|key| roster::shard_prefix(&roster::encode_roster_filename(key)))
        .collect();
    assert!(
        distinct_shards.len() >= 3,
        "precondition failed: expected >=3 distinct shards, got {distinct_shards:?}"
    );

    for key in &keys {
        roster::touch_roster_marker(&fs, key)
            .await
            .expect("seed roster marker");
    }

    let walked = roster::walk_roster_shards(&fs).await;
    for key in &keys {
        assert!(
            walked.contains(key),
            "shard walk must enumerate every seeded scope, including scope in a \
             non-default shard: missing {key:?}"
        );
    }
    assert_eq!(
        walked.len(),
        keys.len(),
        "shard walk must enumerate exactly the seeded scopes, no more, no fewer"
    );
}

// Required test (§5.3, P1.9 extension), integration-tier: lazy-recovery
// admission — a scope with unclosed edges is gated behind
// `ScopeRecoveryInProgress` on first touch, then admitted once recovery
// completes.
#[tokio::test]
async fn scope_with_unclosed_edge_is_recovered_before_new_spawns_are_admitted() {
    let store = real_store();
    let scope = scope(
        "tenant-recover",
        "user-recover",
        Some("agent-recover"),
        None,
    );
    let parent = TurnRunId::new();
    let child = TurnRunId::new();
    // Seed an unclosed (Settled, undrained) edge directly through the
    // store — simulating a prior process leaving one behind.
    store
        .record_awaited_child(test_record(&scope, parent, child))
        .await
        .unwrap();

    let goal_store: Arc<dyn ironclaw_loop_support::SubagentSpawnGoalStore> =
        Arc::new(ironclaw_reborn::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
    let turn_state_store: Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore> =
        Arc::new(InMemoryTurnStateStore::default());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound_deferred_result_writer(
        Arc::clone(&store),
        goal_store,
        turn_state_store,
        thread_service,
    ));
    let driver = ScopeRecoveryDriver::new(Arc::clone(&resolver), Arc::clone(&store));

    use ironclaw_loop_support::AwaitEdgeWriter;
    let first = driver.check_scope_recovered(&scope).await;
    assert!(
        first.is_err(),
        "first touch against a scope with an unclosed edge starts recovery and rejects admission"
    );

    // Recovery runs as a background task; poll until it completes (the
    // production contract is "retryable", not "instant" — callers back
    // off and retry, exactly like `ThreadBusy`).
    let mut admitted = false;
    for _ in 0..200 {
        if driver.check_scope_recovered(&scope).await.is_ok() {
            admitted = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        admitted,
        "scope becomes admissible once its recovery task completes"
    );
}

// Required test (§5.3, P1.9 extension), integration-tier smoke: a scope
// with NO unclosed edges (the common case — a brand-new scope's very first
// spawn) is admitted immediately, never rejected — recovery is not a tax
// on first contact.
#[tokio::test]
async fn brand_new_scope_with_no_unclosed_edges_is_admitted_immediately() {
    let store = real_store();
    let scope = scope("tenant-fresh", "user-fresh", Some("agent-fresh"), None);
    let goal_store: Arc<dyn ironclaw_loop_support::SubagentSpawnGoalStore> =
        Arc::new(ironclaw_reborn::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
    let turn_state_store: Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore> =
        Arc::new(InMemoryTurnStateStore::default());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound_deferred_result_writer(
        Arc::clone(&store),
        goal_store,
        turn_state_store,
        thread_service,
    ));
    let driver = ScopeRecoveryDriver::new(Arc::clone(&resolver), Arc::clone(&store));

    use ironclaw_loop_support::AwaitEdgeWriter;
    assert!(
        driver.check_scope_recovered(&scope).await.is_ok(),
        "a scope with nothing to recover must never be rejected on first touch"
    );
}

fn test_record(
    scope: &TurnScope,
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
) -> ironclaw_loop_support::AwaitedChildSetRecord {
    use ironclaw_loop_support::{AwaitedChildSetRecord, SpawnSubagentMode, SubagentKindId};
    use ironclaw_turns::{
        GateRef, LoopResultRef, ReplyTargetBindingRef, SourceBindingRef,
        run_profile::LoopRunContext,
    };

    let mut parent_run_context =
        ironclaw_agent_loop::test_support::test_run_context("await-edge-integration");
    parent_run_context.scope = scope.clone();
    parent_run_context.run_id = parent_run_id;
    let _: &LoopRunContext = &parent_run_context;

    AwaitedChildSetRecord {
        gate_ref: GateRef::new(format!("gate:subagent-{child_run_id}")).unwrap(),
        parent_run_context,
        tree_root_run_id: parent_run_id,
        child_scope: scope.clone(),
        child_run_id,
        child_thread_id: ThreadId::new("child-thread").unwrap(),
        source_binding_ref: SourceBindingRef::new(format!("subagent-source:{child_run_id}"))
            .unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new(format!(
            "subagent-reply:{child_run_id}"
        ))
        .unwrap(),
        subagent_kind: SubagentKindId::new("general").unwrap(),
        spawn_capability_id: ironclaw_host_api::CapabilityId::new(
            ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
        )
        .unwrap(),
        result_ref: LoopResultRef::new(format!("result:subagent.{child_run_id}")).unwrap(),
        mode: SpawnSubagentMode::Blocking,
    }
}
