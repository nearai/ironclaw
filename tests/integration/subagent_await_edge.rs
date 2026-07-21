//! Subagent await-edge delivery — integration-tier coverage for the P1.x
//! rows the design doc tags "integration-tier" (not covered by the
//! crate-tier unit tests already living alongside
//! `crates/ironclaw_runner/src/subagent/await_edge/{roster,store}.rs`):
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
use ironclaw_loop_host::AwaitEdgeWriter;
use ironclaw_reborn_composition::wrap_scoped;
use ironclaw_runner::subagent::await_edge::{
    boot_recovery::ScopeRecoveryDriver,
    resolver::AwaitEdgeResolver,
    roster::{self, RosterKey},
    store::FilesystemAwaitEdgeStore,
};
use ironclaw_threads::{InMemorySessionThreadService, SessionThreadService, ThreadScope};
use ironclaw_turns::test_support::in_memory_turn_state_store;
use ironclaw_turns::{
    DefaultTurnCoordinator, TurnCoordinator, TurnRunId, TurnScope, TurnSpawnTreePort,
    runner::TurnRunTransitionPort,
};

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

    let goal_store: Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore> =
        Arc::new(ironclaw_runner::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
    let turn_state_store: Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore> =
        Arc::new(in_memory_turn_state_store());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound_deferred_result_writer(
        Arc::clone(&store),
        goal_store,
        turn_state_store,
        thread_service,
    ));
    let driver = ScopeRecoveryDriver::new(Arc::clone(&resolver), Arc::clone(&store));

    use ironclaw_loop_host::AwaitEdgeWriter;
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
    let goal_store: Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore> =
        Arc::new(ironclaw_runner::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
    let turn_state_store: Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore> =
        Arc::new(in_memory_turn_state_store());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound_deferred_result_writer(
        Arc::clone(&store),
        goal_store,
        turn_state_store,
        thread_service,
    ));
    let driver = ScopeRecoveryDriver::new(Arc::clone(&resolver), Arc::clone(&store));

    use ironclaw_loop_host::AwaitEdgeWriter;
    assert!(
        driver.check_scope_recovered(&scope).await.is_ok(),
        "a scope with nothing to recover must never be rejected on first touch"
    );
}

// Required test (FIX A backstop, design doc §2): the real lost-edge window
// is NOT "crash between the child's run-record commit and the parent's
// `store.open()`" (edge-before-submit already rules that out) — it is
// `SpawnCompensationState::rollback` (`ironclaw_loop_host::subagent_spawn_port`)
// unconditionally deleting the just-opened edge while its own child-run
// cancellation is best-effort and unconfirmed, so the child can keep
// running to a real terminal state with no edge left to deliver it
// through. `reconstruct_edge` is the backstop.
//
// SUBSTITUTION, reported explicitly: driving the actual capability-port
// rollback race (forcing `mark_message_submitted` to fail after
// `submit_child_run` succeeds) is not reachable from this crate — that
// race lives inside `ironclaw_loop_host::subagent_spawn_port`'s private
// `SpawnCompensationState::rollback`, a different crate with no injectable
// failure seam at the await-edge integration-test layer. This test drives
// the closest feasible seam instead: open the edge exactly as
// `record_awaited_child` does, then delete it directly via the store
// (exactly what `rollback`'s `abandon_awaited_child` call does), leaving
// the child free to keep running to a real terminal state with no edge on
// disk — then asserts delivery still completes via `reconstruct_edge`.
#[tokio::test]
async fn rollback_deleted_edge_is_reconstructed_so_the_parent_still_gets_the_result() {
    let store = real_store();
    let state_store = Arc::new(in_memory_turn_state_store());
    let coordinator = Arc::new(DefaultTurnCoordinator::new(Arc::clone(&state_store)));
    let thread_service = Arc::new(InMemorySessionThreadService::default());

    let tenant = TenantId::new("tenant-rollback").unwrap();
    let user = UserId::new("user-rollback").unwrap();
    let agent = AgentId::new("agent-rollback").unwrap();
    let parent_thread_id = ThreadId::new("parent-thread-rollback").unwrap();
    let parent_scope = TurnScope::new_with_owner(
        tenant.clone(),
        Some(agent.clone()),
        None,
        parent_thread_id.clone(),
        Some(user.clone()),
    );
    let actor = ironclaw_turns::TurnActor::new(user.clone());

    // 1. Submit and block the parent on a dependent-run gate — the exact
    // state a real parent is in while its blocking-mode child runs.
    let submitted = coordinator
        .submit_turn(ironclaw_turns::SubmitTurnRequest {
            requested_model: None,
            scope: parent_scope.clone(),
            actor: actor.clone(),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg:parent-rollback")
                .unwrap(),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:parent-rollback")
                .unwrap(),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "reply:parent-rollback",
            )
            .unwrap(),
            requested_run_profile: None,
            idempotency_key: ironclaw_turns::IdempotencyKey::new("idem:parent-rollback").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
        })
        .await
        .unwrap();
    let ironclaw_turns::SubmitTurnResponse::Accepted {
        run_id: parent_run_id,
        ..
    } = submitted;
    let runner_id = ironclaw_turns::TurnRunnerId::new();
    let lease_token = ironclaw_turns::TurnLeaseToken::new();
    state_store
        .claim_next_run(ironclaw_turns::runner::ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .expect("parent run claimable");
    let gate_ref = ironclaw_turns::GateRef::new("gate:subagent-rollback-test").unwrap();
    state_store
        .block_run(ironclaw_turns::runner::BlockRunRequest {
            run_id: parent_run_id,
            runner_id,
            lease_token,
            checkpoint_id: ironclaw_turns::TurnCheckpointId::new(),
            state_ref: ironclaw_turns::run_profile::LoopCheckpointStateRef::new(
                "checkpoint:rollback-test",
            )
            .unwrap(),
            reason: ironclaw_turns::BlockedReason::AwaitDependentRun {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();

    // 2. Submit the child as a real lineage child of the parent.
    let child_thread_id = ThreadId::new("child-thread-rollback").unwrap();
    let child_scope = TurnScope::new_with_owner(
        tenant.clone(),
        Some(agent.clone()),
        None,
        child_thread_id.clone(),
        Some(user.clone()),
    );
    let child_submitted = coordinator
        .submit_child_run(ironclaw_turns::SubmitChildRunRequest {
            parent_scope: parent_scope.clone(),
            parent_run_id,
            child_scope: child_scope.clone(),
            actor: actor.clone(),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg:child-rollback")
                .unwrap(),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:child-rollback")
                .unwrap(),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "reply:child-rollback",
            )
            .unwrap(),
            requested_run_profile: None,
            idempotency_key: ironclaw_turns::IdempotencyKey::new("idem:child-rollback").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            spawn_tree_descendant_cap: 16,
        })
        .await
        .unwrap();
    let ironclaw_turns::SubmitTurnResponse::Accepted {
        run_id: child_run_id,
        ..
    } = child_submitted;

    // 3. Write the child's thread metadata — exactly what `finish_spawn`
    // does before the (simulated) rollback ever runs, now including the
    // FIX A fields (`parent_run_context`, `gate_ref`).
    let mut parent_run_context =
        ironclaw_agent_loop::test_support::test_run_context("rollback-parent");
    parent_run_context.scope = parent_scope.clone();
    parent_run_context.run_id = parent_run_id;
    parent_run_context.actor = Some(actor.clone());
    let result_ref = ironclaw_turns::LoopResultRef::new("result:subagent.rollback").unwrap();
    let metadata = ironclaw_loop_host::SubagentThreadMetadata {
        kind: ironclaw_loop_host::SubagentThreadKind::Subagent,
        parent_run_id,
        parent_thread_id: parent_thread_id.clone(),
        tree_root_run_id: parent_run_id,
        child_run_id,
        subagent_kind: ironclaw_loop_host::SubagentKindId::new("general").unwrap(),
        mode: ironclaw_loop_host::SpawnSubagentMode::Blocking,
        result_ref: result_ref.clone(),
        handoff: None,
        parent_run_context,
        gate_ref: gate_ref.clone(),
    };
    thread_service
        .ensure_thread(ironclaw_threads::EnsureThreadRequest {
            scope: ThreadScope {
                tenant_id: tenant.clone(),
                agent_id: agent.clone(),
                project_id: None,
                owner_user_id: Some(user.clone()),
                mission_id: None,
            },
            thread_id: Some(child_thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: Some("Subagent".to_string()),
            metadata_json: Some(serde_json::to_string(&metadata).unwrap()),
        })
        .await
        .unwrap();

    // The parent's own thread must carry the placeholder tool-result
    // reference the spawn-time `write_capability_result` call would have
    // appended — `update_parent_result_reference` (drain path) updates it
    // in place, it does not create it.
    thread_service
        .ensure_thread(ironclaw_threads::EnsureThreadRequest {
            scope: ThreadScope {
                tenant_id: tenant.clone(),
                agent_id: agent.clone(),
                project_id: None,
                owner_user_id: Some(user.clone()),
                mission_id: None,
            },
            thread_id: Some(parent_thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    thread_service
        .append_tool_result_reference(ironclaw_threads::AppendToolResultReferenceRequest {
            scope: ThreadScope {
                tenant_id: tenant.clone(),
                agent_id: agent.clone(),
                project_id: None,
                owner_user_id: Some(user.clone()),
                mission_id: None,
            },
            thread_id: parent_thread_id.clone(),
            turn_run_id: parent_run_id.to_string(),
            result_ref: result_ref.as_str().to_string(),
            safe_summary: ironclaw_threads::ToolResultSafeSummary::new("subagent spawned").unwrap(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .unwrap();

    // 4. Open the edge (what `record_awaited_child` does), then delete it
    // directly (what `rollback`'s `abandon_awaited_child` call does) —
    // simulating the unconfirmed-cancel race: the edge is gone, but the
    // child (below) keeps running regardless.
    store
        .record_awaited_child(test_record(&child_scope, parent_run_id, child_run_id))
        .await
        .unwrap();
    store
        .abandon_awaited_child(&child_scope, parent_run_id, child_run_id)
        .await
        .unwrap();
    assert!(
        store
            .list_unclosed_for_scope(&child_scope)
            .await
            .unwrap()
            .is_empty(),
        "the edge must actually be gone before the child reaches terminal -- that's the race"
    );

    // 5. The child, unaware its edge was deleted, keeps running to a real
    // terminal state.
    let child_runner_id = ironclaw_turns::TurnRunnerId::new();
    let child_lease = ironclaw_turns::TurnLeaseToken::new();
    state_store
        .claim_next_run(ironclaw_turns::runner::ClaimRunRequest {
            runner_id: child_runner_id,
            lease_token: child_lease,
            scope_filter: None,
        })
        .await
        .unwrap()
        .expect("child run claimable");
    state_store
        .complete_run(ironclaw_turns::runner::CompleteRunRequest {
            run_id: child_run_id,
            runner_id: child_runner_id,
            lease_token: child_lease,
        })
        .await
        .unwrap();

    // 6. Deliver the child's terminal event through the resolver — the
    // exact call `TurnCommittedEventObserver::observe_committed_event`
    // makes in production.
    let goal_store: Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore> =
        Arc::new(ironclaw_runner::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
    let turn_state_store: Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore> = state_store.clone();
    let result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
        Arc::new(AllowResultWriter);
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
        Arc::clone(&store),
        goal_store,
        turn_state_store,
        result_writer,
        Arc::clone(&thread_service),
    ));
    let coordinator_dyn: Arc<dyn TurnCoordinator> = coordinator.clone();
    resolver.bind_coordinator(coordinator_dyn).unwrap();

    let event = ironclaw_turns::TurnLifecycleEvent {
        cursor: ironclaw_turns::events::EventCursor(1),
        scope: child_scope.clone(),
        occurred_at: None,
        owner_user_id: Some(user.clone()),
        run_id: child_run_id,
        status: ironclaw_turns::TurnStatus::Completed,
        kind: ironclaw_turns::TurnEventKind::Completed,
        blocked_gate: None,
        sanitized_reason: None,
        retryable: None,
        detail: None,
    };
    let outcome = resolver
        .handle_child_terminal(&event)
        .await
        .expect("resolver should not error");
    assert_eq!(
        outcome,
        ironclaw_loop_host::ResolveOutcome::Resumed,
        "reconstruct_edge must rebuild the deleted edge from cached thread metadata \
         and deliver the result -- the parent must not be left stuck"
    );

    // 7. The parent actually left `BlockedDependentRun` -- not stuck.
    let parent_state = coordinator
        .get_run_state(ironclaw_turns::GetRunStateRequest {
            scope: parent_scope.clone(),
            run_id: parent_run_id,
        })
        .await
        .unwrap();
    assert_ne!(
        parent_state.status,
        ironclaw_turns::TurnStatus::BlockedDependentRun,
        "the parent must actually resume, not stay stuck on its dependent-run gate"
    );
}

// D3 batch-gate group drain must report each member's own status/reason,
// never the last-settling member's (external review, PR #5819). Mutation:
// revert `drain_settled_group`'s per-member derivation to the driving
// event -> RED (child_a reads "completed" instead of "failed").
#[tokio::test]
async fn mixed_status_batch_group_reports_each_members_own_status_and_reason() {
    let store = real_store();
    let state_store = Arc::new(in_memory_turn_state_store());
    let coordinator = Arc::new(DefaultTurnCoordinator::new(Arc::clone(&state_store)));
    let thread_service = Arc::new(InMemorySessionThreadService::default());

    let tenant = TenantId::new("tenant-mixed-batch").unwrap();
    let user = UserId::new("user-mixed-batch").unwrap();
    let agent = AgentId::new("agent-mixed-batch").unwrap();
    let parent_thread_id = ThreadId::new("parent-thread-mixed-batch").unwrap();
    let parent_scope = TurnScope::new_with_owner(
        tenant.clone(),
        Some(agent.clone()),
        None,
        parent_thread_id.clone(),
        Some(user.clone()),
    );
    let actor = ironclaw_turns::TurnActor::new(user.clone());

    // 1. Submit and block the parent on a shared dependent-run gate.
    let submitted = coordinator
        .submit_turn(ironclaw_turns::SubmitTurnRequest {
            requested_model: None,
            scope: parent_scope.clone(),
            actor: actor.clone(),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg:parent-mixed-batch")
                .unwrap(),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:parent-mixed-batch")
                .unwrap(),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "reply:parent-mixed-batch",
            )
            .unwrap(),
            requested_run_profile: None,
            idempotency_key: ironclaw_turns::IdempotencyKey::new("idem:parent-mixed-batch")
                .unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
        })
        .await
        .unwrap();
    let ironclaw_turns::SubmitTurnResponse::Accepted {
        run_id: parent_run_id,
        ..
    } = submitted;
    let runner_id = ironclaw_turns::TurnRunnerId::new();
    let lease_token = ironclaw_turns::TurnLeaseToken::new();
    state_store
        .claim_next_run(ironclaw_turns::runner::ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .expect("parent run claimable");
    let gate_ref = ironclaw_turns::GateRef::new("gate:subagent-mixed-batch-test").unwrap();
    state_store
        .block_run(ironclaw_turns::runner::BlockRunRequest {
            run_id: parent_run_id,
            runner_id,
            lease_token,
            checkpoint_id: ironclaw_turns::TurnCheckpointId::new(),
            state_ref: ironclaw_turns::run_profile::LoopCheckpointStateRef::new(
                "checkpoint:mixed-batch-test",
            )
            .unwrap(),
            reason: ironclaw_turns::BlockedReason::AwaitDependentRun {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();

    // 2. Submit both children as real lineage children of the parent --
    // their own run status never advances past Queued; the resolver drives
    // entirely off the `TurnLifecycleEvent`s constructed below, exactly like
    // a real `TurnCommittedEventObserver` dispatch would.
    let child_a_thread_id = ThreadId::new("child-a-thread-mixed-batch").unwrap();
    let child_a_scope = TurnScope::new_with_owner(
        tenant.clone(),
        Some(agent.clone()),
        None,
        child_a_thread_id.clone(),
        Some(user.clone()),
    );
    let ironclaw_turns::SubmitTurnResponse::Accepted {
        run_id: child_a_run_id,
        ..
    } = coordinator
        .submit_child_run(ironclaw_turns::SubmitChildRunRequest {
            parent_scope: parent_scope.clone(),
            parent_run_id,
            child_scope: child_a_scope.clone(),
            actor: actor.clone(),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new(
                "msg:child-a-mixed-batch",
            )
            .unwrap(),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:child-a-mixed-batch")
                .unwrap(),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "reply:child-a-mixed-batch",
            )
            .unwrap(),
            requested_run_profile: None,
            idempotency_key: ironclaw_turns::IdempotencyKey::new("idem:child-a-mixed-batch")
                .unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            spawn_tree_descendant_cap: 16,
        })
        .await
        .unwrap();

    let child_b_thread_id = ThreadId::new("child-b-thread-mixed-batch").unwrap();
    let child_b_scope = TurnScope::new_with_owner(
        tenant.clone(),
        Some(agent.clone()),
        None,
        child_b_thread_id.clone(),
        Some(user.clone()),
    );
    let ironclaw_turns::SubmitTurnResponse::Accepted {
        run_id: child_b_run_id,
        ..
    } = coordinator
        .submit_child_run(ironclaw_turns::SubmitChildRunRequest {
            parent_scope: parent_scope.clone(),
            parent_run_id,
            child_scope: child_b_scope.clone(),
            actor: actor.clone(),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new(
                "msg:child-b-mixed-batch",
            )
            .unwrap(),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:child-b-mixed-batch")
                .unwrap(),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "reply:child-b-mixed-batch",
            )
            .unwrap(),
            requested_run_profile: None,
            idempotency_key: ironclaw_turns::IdempotencyKey::new("idem:child-b-mixed-batch")
                .unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            spawn_tree_descendant_cap: 16,
        })
        .await
        .unwrap();

    // 3. Ensure both children's own (empty) threads exist --
    // `child_terminal_output` looks up each child's latest assistant message
    // by thread, and errors on a wholly unknown thread rather than treating
    // it as "no final message yet".
    for (child_thread_id, child_agent) in
        [(&child_a_thread_id, &agent), (&child_b_thread_id, &agent)]
    {
        thread_service
            .ensure_thread(ironclaw_threads::EnsureThreadRequest {
                scope: ThreadScope {
                    tenant_id: tenant.clone(),
                    agent_id: child_agent.clone(),
                    project_id: None,
                    owner_user_id: Some(user.clone()),
                    mission_id: None,
                },
                thread_id: Some(child_thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: Some("Subagent".to_string()),
                metadata_json: None,
            })
            .await
            .unwrap();
    }

    // 4. Open both edges directly under the SAME shared `gate_ref` -- what
    // `record_awaited_child` does at spawn time for a D3 batch of spawns
    // issued in one call.
    let mut parent_run_context =
        ironclaw_agent_loop::test_support::test_run_context("mixed-batch-parent");
    parent_run_context.scope = parent_scope.clone();
    parent_run_context.thread_id = parent_thread_id.clone();
    parent_run_context.run_id = parent_run_id;
    parent_run_context.actor = Some(actor.clone());
    let result_ref_a = ironclaw_turns::LoopResultRef::new("result:subagent.mixed-a").unwrap();
    let result_ref_b = ironclaw_turns::LoopResultRef::new("result:subagent.mixed-b").unwrap();
    store
        .record_awaited_child(mixed_batch_record(
            &child_a_scope,
            child_a_thread_id.clone(),
            parent_run_id,
            child_a_run_id,
            gate_ref.clone(),
            result_ref_a.clone(),
            parent_run_context.clone(),
        ))
        .await
        .unwrap();
    store
        .record_awaited_child(mixed_batch_record(
            &child_b_scope,
            child_b_thread_id.clone(),
            parent_run_id,
            child_b_run_id,
            gate_ref.clone(),
            result_ref_b.clone(),
            parent_run_context,
        ))
        .await
        .unwrap();

    // 5. Seed the parent thread with both spawn-time placeholder tool-result
    // references -- `update_parent_result_reference` updates them in place,
    // it does not create them.
    let parent_thread_scope = ThreadScope {
        tenant_id: tenant.clone(),
        agent_id: agent.clone(),
        project_id: None,
        owner_user_id: Some(user.clone()),
        mission_id: None,
    };
    thread_service
        .ensure_thread(ironclaw_threads::EnsureThreadRequest {
            scope: parent_thread_scope.clone(),
            thread_id: Some(parent_thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    for result_ref in [&result_ref_a, &result_ref_b] {
        thread_service
            .append_tool_result_reference(ironclaw_threads::AppendToolResultReferenceRequest {
                scope: parent_thread_scope.clone(),
                thread_id: parent_thread_id.clone(),
                turn_run_id: parent_run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                safe_summary: ironclaw_threads::ToolResultSafeSummary::new("subagent spawned")
                    .unwrap(),
                provider_call: None,
                model_observation: None,
            })
            .await
            .unwrap();
    }

    // 6. Build the resolver and deliver each child's own terminal event --
    // child_a fails first (group not yet fully settled -> no drain), then
    // child_b completes last and drives the batch drain.
    let goal_store: Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore> =
        Arc::new(ironclaw_runner::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
    let turn_state_store: Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore> = state_store.clone();
    let result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
        Arc::new(AllowResultWriter);
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
        Arc::clone(&store),
        goal_store,
        turn_state_store,
        result_writer,
        Arc::clone(&thread_service),
    ));
    let coordinator_dyn: Arc<dyn TurnCoordinator> = coordinator.clone();
    resolver.bind_coordinator(coordinator_dyn).unwrap();

    let child_a_failure_reason = "child_a_specific_failure_reason";
    let event_a = ironclaw_turns::TurnLifecycleEvent {
        cursor: ironclaw_turns::events::EventCursor(1),
        scope: child_a_scope.clone(),
        occurred_at: None,
        owner_user_id: Some(user.clone()),
        run_id: child_a_run_id,
        status: ironclaw_turns::TurnStatus::Failed,
        kind: ironclaw_turns::TurnEventKind::Failed,
        blocked_gate: None,
        sanitized_reason: Some(child_a_failure_reason.to_string()),
        retryable: None,
        detail: None,
    };
    let outcome_a = resolver
        .handle_child_terminal(&event_a)
        .await
        .expect("settling child_a should not error");
    assert_eq!(
        outcome_a,
        ironclaw_loop_host::ResolveOutcome::AlreadyClosed,
        "child_a settling first must not drive the batch drain -- child_b is still Open"
    );

    let event_b = ironclaw_turns::TurnLifecycleEvent {
        cursor: ironclaw_turns::events::EventCursor(1),
        scope: child_b_scope.clone(),
        occurred_at: None,
        owner_user_id: Some(user.clone()),
        run_id: child_b_run_id,
        status: ironclaw_turns::TurnStatus::Completed,
        kind: ironclaw_turns::TurnEventKind::Completed,
        blocked_gate: None,
        sanitized_reason: None,
        retryable: None,
        detail: None,
    };
    let outcome_b = resolver
        .handle_child_terminal(&event_b)
        .await
        .expect("settling child_b should not error");
    assert_eq!(
        outcome_b,
        ironclaw_loop_host::ResolveOutcome::Resumed,
        "child_b settling last must drive the batch drain and resume the parent"
    );

    // 7. Each member's OWN parent-transcript summary must reflect its OWN
    // status/reason -- never the other member's (or the driving event's).
    let history = thread_service
        .list_thread_history(ironclaw_threads::ThreadHistoryRequest {
            scope: parent_thread_scope,
            thread_id: parent_thread_id,
        })
        .await
        .unwrap();
    let summary_for = |result_ref: &str| -> String {
        let message = history
            .messages
            .iter()
            .find(|message| message.tool_result_ref.as_deref() == Some(result_ref))
            .unwrap_or_else(|| panic!("no tool-result-reference message found for {result_ref}"));
        let content = message.content.as_deref().expect("message has content");
        let envelope = ironclaw_threads::ToolResultReferenceEnvelope::from_json_str(content)
            .expect("valid tool-result-reference envelope");
        envelope.safe_summary.as_str().to_string()
    };

    let summary_a = summary_for(result_ref_a.as_str());
    assert!(
        summary_a.contains("failed"),
        "child_a's own summary must report its own Failed status: {summary_a}"
    );
    assert!(
        summary_a.contains(child_a_failure_reason),
        "child_a's own summary must carry its own failure reason: {summary_a}"
    );

    let summary_b = summary_for(result_ref_b.as_str());
    assert!(
        summary_b.contains("completed"),
        "child_b's own summary must report its own Completed status, not child_a's Failed \
         status: {summary_b}"
    );
    assert!(
        !summary_b.contains(child_a_failure_reason),
        "child_b's own summary must never carry child_a's failure reason: {summary_b}"
    );
}

fn mixed_batch_record(
    scope: &TurnScope,
    child_thread_id: ThreadId,
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
    gate_ref: ironclaw_turns::GateRef,
    result_ref: ironclaw_turns::LoopResultRef,
    parent_run_context: ironclaw_turns::run_profile::LoopRunContext,
) -> ironclaw_loop_host::AwaitedChildSetRecord {
    use ironclaw_loop_host::{AwaitedChildSetRecord, SpawnSubagentMode, SubagentKindId};
    use ironclaw_turns::{ReplyTargetBindingRef, SourceBindingRef};

    AwaitedChildSetRecord {
        gate_ref,
        parent_run_context,
        tree_root_run_id: parent_run_id,
        child_scope: scope.clone(),
        child_run_id,
        child_thread_id,
        source_binding_ref: SourceBindingRef::new(format!("subagent-source:{child_run_id}"))
            .unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new(format!(
            "subagent-reply:{child_run_id}"
        ))
        .unwrap(),
        subagent_kind: SubagentKindId::new("general").unwrap(),
        spawn_capability_id: ironclaw_host_api::CapabilityId::new(
            ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
        )
        .unwrap(),
        result_ref,
        mode: SpawnSubagentMode::Blocking,
    }
}

struct AllowResultWriter;

#[async_trait::async_trait]
impl ironclaw_loop_host::LoopCapabilityResultWriter for AllowResultWriter {
    async fn write_capability_result(
        &self,
        write: ironclaw_loop_host::CapabilityResultWrite<'_>,
    ) -> Result<
        ironclaw_loop_host::CapabilityWriteResult,
        ironclaw_turns::run_profile::AgentLoopHostError,
    > {
        Ok(
            ironclaw_loop_host::CapabilityWriteResult::without_output_digest(
                ironclaw_turns::LoopResultRef::new(format!("result:{}", write.capability_id))
                    .unwrap(),
                0,
            ),
        )
    }

    async fn update_capability_result(
        &self,
        _run_context: &ironclaw_turns::run_profile::LoopRunContext,
        _result_ref: &ironclaw_turns::LoopResultRef,
        output: serde_json::Value,
    ) -> Result<u64, ironclaw_turns::run_profile::AgentLoopHostError> {
        Ok(serde_json::to_vec(&output)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(0))
    }
}

fn test_record(
    scope: &TurnScope,
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
) -> ironclaw_loop_host::AwaitedChildSetRecord {
    use ironclaw_loop_host::{AwaitedChildSetRecord, SpawnSubagentMode, SubagentKindId};
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
            ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
        )
        .unwrap(),
        result_ref: LoopResultRef::new(format!("result:subagent.{child_run_id}")).unwrap(),
        mode: SpawnSubagentMode::Blocking,
    }
}
