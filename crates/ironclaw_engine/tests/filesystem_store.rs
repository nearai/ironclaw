//! Contract test suite for [`FilesystemStore`].
//!
//! Each test drives the public [`Store`] surface so a regression in any
//! method shape (path encoding, indexed projection, CAS, status filter) is
//! caught by the same suite as the in-memory reference implementation.
//!
//! Tests use the [`InMemoryBackend`] so the suite is hermetic — no
//! Postgres / libSQL needed. The same `FilesystemStore<F>` is generic
//! over [`RootFilesystem`], so when this passes against the in-memory
//! backend it also exercises the full `put`/`get`/`query`/`ensure_index`
//! shape that production backends serve.

use std::sync::Arc;

use ironclaw_engine::types::capability::{CapabilityLease, GrantedActions, LeaseId};
use ironclaw_engine::types::conversation::ConversationSurface;
use ironclaw_engine::types::memory::{DocType, MemoryDoc};
use ironclaw_engine::types::mission::{Mission, MissionCadence, MissionStatus};
use ironclaw_engine::types::project::Project;
use ironclaw_engine::types::step::Step;
use ironclaw_engine::types::thread::{Thread, ThreadConfig, ThreadId, ThreadState, ThreadType};
use ironclaw_engine::types::{LEGACY_SHARED_OWNER_ID, shared_owner_id};
use ironclaw_engine::{EventKind, FilesystemStore, ProjectId, Store, ThreadEvent};
use ironclaw_filesystem::InMemoryBackend;

fn make_store() -> FilesystemStore<InMemoryBackend> {
    FilesystemStore::new(Arc::new(InMemoryBackend::new()))
}

fn make_thread(project_id: ProjectId, user_id: &str) -> Thread {
    Thread::new(
        "test goal",
        ThreadType::Foreground,
        project_id,
        user_id,
        ThreadConfig::default(),
    )
}

#[tokio::test]
async fn save_and_load_thread_round_trips() {
    let store = make_store();
    let thread = make_thread(ProjectId::new(), "alice");
    let id = thread.id;
    store.save_thread(&thread).await.unwrap();

    let loaded = store.load_thread(id).await.unwrap().expect("present");
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.user_id, "alice");
    assert_eq!(loaded.goal, "test goal");
}

#[tokio::test]
async fn load_thread_missing_returns_none() {
    let store = make_store();
    assert!(store.load_thread(ThreadId::new()).await.unwrap().is_none());
}

#[tokio::test]
async fn list_threads_filters_by_project_and_user() {
    let store = make_store();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();

    let alice_a = make_thread(project_a, "alice");
    let alice_b = make_thread(project_b, "alice");
    let bob_a = make_thread(project_a, "bob");

    store.save_thread(&alice_a).await.unwrap();
    store.save_thread(&alice_b).await.unwrap();
    store.save_thread(&bob_a).await.unwrap();

    let alice_in_a = store.list_threads(project_a, "alice").await.unwrap();
    assert_eq!(alice_in_a.len(), 1);
    assert_eq!(alice_in_a[0].id, alice_a.id);

    let bob_in_a = store.list_threads(project_a, "bob").await.unwrap();
    assert_eq!(bob_in_a.len(), 1);
    assert_eq!(bob_in_a[0].id, bob_a.id);

    let alice_in_b = store.list_threads(project_b, "alice").await.unwrap();
    assert_eq!(alice_in_b.len(), 1);
    assert_eq!(alice_in_b[0].id, alice_b.id);
}

#[tokio::test]
async fn update_thread_state_persists_via_cas_path() {
    let store = make_store();
    let mut thread = make_thread(ProjectId::new(), "alice");
    let id = thread.id;
    // Move to Running so the next transition is valid.
    thread.state = ThreadState::Created;
    store.save_thread(&thread).await.unwrap();

    store
        .update_thread_state(id, ThreadState::Running)
        .await
        .unwrap();

    let reloaded = store.load_thread(id).await.unwrap().expect("present");
    assert_eq!(reloaded.state, ThreadState::Running);
}

#[tokio::test]
async fn update_thread_state_silently_skips_unknown_id() {
    let store = make_store();
    // Must not error — matches HybridStore behaviour for unknown ids.
    store
        .update_thread_state(ThreadId::new(), ThreadState::Running)
        .await
        .unwrap();
}

#[tokio::test]
async fn list_all_threads_returns_all_users_in_project() {
    let store = make_store();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();

    let alice = make_thread(project_a, "alice");
    let bob = make_thread(project_a, "bob");
    let charlie = make_thread(project_b, "charlie");
    store.save_thread(&alice).await.unwrap();
    store.save_thread(&bob).await.unwrap();
    store.save_thread(&charlie).await.unwrap();

    let mut in_a = store.list_all_threads(project_a).await.unwrap();
    in_a.sort_by(|a, b| a.user_id.cmp(&b.user_id));
    assert_eq!(in_a.len(), 2);
    assert_eq!(in_a[0].user_id, "alice");
    assert_eq!(in_a[1].user_id, "bob");
}

#[tokio::test]
async fn save_and_load_steps_orders_by_sequence() {
    let store = make_store();
    let thread_id = ThreadId::new();
    let mut s1 = Step::new(thread_id, 1);
    s1.sequence = 1;
    let mut s2 = Step::new(thread_id, 2);
    s2.sequence = 2;
    let mut s3 = Step::new(thread_id, 3);
    s3.sequence = 3;

    // Save out of order — load should return them sorted.
    store.save_step(&s3).await.unwrap();
    store.save_step(&s1).await.unwrap();
    store.save_step(&s2).await.unwrap();

    let loaded = store.load_steps(thread_id).await.unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].sequence, 1);
    assert_eq!(loaded[1].sequence, 2);
    assert_eq!(loaded[2].sequence, 3);
}

#[tokio::test]
async fn load_steps_for_unknown_thread_returns_empty() {
    let store = make_store();
    assert!(store.load_steps(ThreadId::new()).await.unwrap().is_empty());
}

#[tokio::test]
async fn append_and_load_events_orders_by_timestamp() {
    let store = make_store();
    let thread_id = ThreadId::new();
    let e1 = ThreadEvent::new(
        thread_id,
        EventKind::StateChanged {
            from: ThreadState::Created,
            to: ThreadState::Running,
            reason: None,
        },
    );
    // Slight delay so timestamps differ.
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let e2 = ThreadEvent::new(
        thread_id,
        EventKind::StateChanged {
            from: ThreadState::Running,
            to: ThreadState::Completed,
            reason: None,
        },
    );

    store
        .append_events(&[e2.clone(), e1.clone()])
        .await
        .unwrap();
    let loaded = store.load_events(thread_id).await.unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].id, e1.id);
    assert_eq!(loaded[1].id, e2.id);
}

#[tokio::test]
async fn save_and_load_project_round_trips() {
    let store = make_store();
    let project = Project::new("alice", "commitments", "");
    let id = project.id;
    store.save_project(&project).await.unwrap();

    let loaded = store.load_project(id).await.unwrap().expect("present");
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.user_id, "alice");
}

#[tokio::test]
async fn list_projects_filters_by_user() {
    let store = make_store();
    let p1 = Project::new("alice", "one", "");
    let p2 = Project::new("alice", "two", "");
    let p3 = Project::new("bob", "three", "");
    store.save_project(&p1).await.unwrap();
    store.save_project(&p2).await.unwrap();
    store.save_project(&p3).await.unwrap();

    let alice = store.list_projects("alice").await.unwrap();
    assert_eq!(alice.len(), 2);
    let bob = store.list_projects("bob").await.unwrap();
    assert_eq!(bob.len(), 1);

    let all = store.list_all_projects().await.unwrap();
    assert_eq!(all.len(), 3);
}

#[tokio::test]
async fn save_and_load_conversation_round_trips() {
    let store = make_store();
    let conversation = ConversationSurface::new("web", "alice");
    let id = conversation.id;
    store.save_conversation(&conversation).await.unwrap();

    let loaded = store.load_conversation(id).await.unwrap().expect("present");
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.user_id, "alice");
}

#[tokio::test]
async fn list_conversations_filters_by_user() {
    let store = make_store();
    let alice = ConversationSurface::new("web", "alice");
    let bob = ConversationSurface::new("web", "bob");
    store.save_conversation(&alice).await.unwrap();
    store.save_conversation(&bob).await.unwrap();

    let alice_list = store.list_conversations("alice").await.unwrap();
    assert_eq!(alice_list.len(), 1);
    assert_eq!(alice_list[0].id, alice.id);
}

#[tokio::test]
async fn save_and_load_memory_doc_round_trips() {
    let store = make_store();
    let project_id = ProjectId::new();
    let doc = MemoryDoc::new(project_id, "alice", DocType::Note, "hello", "world");
    let id = doc.id;
    store.save_memory_doc(&doc).await.unwrap();

    let loaded = store.load_memory_doc(id).await.unwrap().expect("present");
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.title, "hello");
}

#[tokio::test]
async fn list_memory_docs_filters_by_project_and_user() {
    let store = make_store();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();

    let alice_a = MemoryDoc::new(project_a, "alice", DocType::Note, "a-note", "");
    let alice_b = MemoryDoc::new(project_b, "alice", DocType::Note, "b-note", "");
    let bob_a = MemoryDoc::new(project_a, "bob", DocType::Note, "bob-note", "");
    store.save_memory_doc(&alice_a).await.unwrap();
    store.save_memory_doc(&alice_b).await.unwrap();
    store.save_memory_doc(&bob_a).await.unwrap();

    let alice_in_a = store.list_memory_docs(project_a, "alice").await.unwrap();
    assert_eq!(alice_in_a.len(), 1);
    assert_eq!(alice_in_a[0].id, alice_a.id);
}

#[tokio::test]
async fn list_memory_docs_with_shared_includes_shared_owner() {
    let store = make_store();
    let project_id = ProjectId::new();
    let alice_doc = MemoryDoc::new(project_id, "alice", DocType::Note, "alice", "");
    let shared = MemoryDoc::new(project_id, shared_owner_id(), DocType::Skill, "shared", "");
    let legacy_shared = MemoryDoc::new(
        project_id,
        LEGACY_SHARED_OWNER_ID,
        DocType::Skill,
        "legacy",
        "",
    );
    store.save_memory_doc(&alice_doc).await.unwrap();
    store.save_memory_doc(&shared).await.unwrap();
    store.save_memory_doc(&legacy_shared).await.unwrap();

    let docs = store
        .list_memory_docs_with_shared(project_id, "alice")
        .await
        .unwrap();
    assert_eq!(docs.len(), 3);
}

#[tokio::test]
async fn list_memory_docs_by_owner_finds_cross_project() {
    let store = make_store();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();
    let in_a = MemoryDoc::new(project_a, "alice", DocType::Note, "in-a", "");
    let in_b = MemoryDoc::new(project_b, "alice", DocType::Note, "in-b", "");
    store.save_memory_doc(&in_a).await.unwrap();
    store.save_memory_doc(&in_b).await.unwrap();

    let all = store.list_memory_docs_by_owner("alice").await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn save_and_load_active_leases_filters_revoked() {
    let store = make_store();
    let thread_id = ThreadId::new();
    let active = CapabilityLease {
        id: LeaseId::new(),
        thread_id,
        capability_name: "test".into(),
        granted_actions: GrantedActions::All,
        granted_at: chrono::Utc::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        revoked: false,
        revoked_reason: None,
    };
    let revoked = CapabilityLease {
        id: LeaseId::new(),
        thread_id,
        capability_name: "test2".into(),
        granted_actions: GrantedActions::All,
        granted_at: chrono::Utc::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        revoked: true,
        revoked_reason: Some("test".into()),
    };
    store.save_lease(&active).await.unwrap();
    store.save_lease(&revoked).await.unwrap();

    let loaded = store.load_active_leases(thread_id).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, active.id);
}

#[tokio::test]
async fn revoke_lease_persists_state_change() {
    let store = make_store();
    let thread_id = ThreadId::new();
    let lease = CapabilityLease {
        id: LeaseId::new(),
        thread_id,
        capability_name: "test".into(),
        granted_actions: GrantedActions::All,
        granted_at: chrono::Utc::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        revoked: false,
        revoked_reason: None,
    };
    store.save_lease(&lease).await.unwrap();

    store.revoke_lease(lease.id, "test reason").await.unwrap();

    let active = store.load_active_leases(thread_id).await.unwrap();
    assert!(active.is_empty(), "revoked lease must drop out of active");
}

#[tokio::test]
async fn save_and_load_mission_round_trips() {
    let store = make_store();
    let project_id = ProjectId::new();
    let mission = Mission::new(
        project_id,
        "alice",
        "mission-a",
        "achieve x",
        MissionCadence::Manual,
    );
    let id = mission.id;
    store.save_mission(&mission).await.unwrap();

    let loaded = store.load_mission(id).await.unwrap().expect("present");
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.user_id, "alice");
}

#[tokio::test]
async fn list_missions_filters_by_project_and_user() {
    let store = make_store();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();

    let alice_a = Mission::new(project_a, "alice", "a", "goal", MissionCadence::Manual);
    let alice_b = Mission::new(project_b, "alice", "b", "goal", MissionCadence::Manual);
    let bob_a = Mission::new(project_a, "bob", "c", "goal", MissionCadence::Manual);
    store.save_mission(&alice_a).await.unwrap();
    store.save_mission(&alice_b).await.unwrap();
    store.save_mission(&bob_a).await.unwrap();

    let alice_in_a = store.list_missions(project_a, "alice").await.unwrap();
    assert_eq!(alice_in_a.len(), 1);
    assert_eq!(alice_in_a[0].id, alice_a.id);

    let all_in_a = store.list_all_missions(project_a).await.unwrap();
    assert_eq!(all_in_a.len(), 2);
}

#[tokio::test]
async fn update_mission_status_persists() {
    let store = make_store();
    let project_id = ProjectId::new();
    let mission = Mission::new(project_id, "alice", "m", "goal", MissionCadence::Manual);
    store.save_mission(&mission).await.unwrap();

    store
        .update_mission_status(mission.id, MissionStatus::Paused)
        .await
        .unwrap();

    let loaded = store
        .load_mission(mission.id)
        .await
        .unwrap()
        .expect("present");
    assert_eq!(loaded.status, MissionStatus::Paused);
}

#[tokio::test]
async fn list_shared_missions_includes_both_owner_aliases() {
    let store = make_store();
    let project_id = ProjectId::new();
    let current = Mission::new(
        project_id,
        shared_owner_id(),
        "current",
        "goal",
        MissionCadence::Manual,
    );
    let legacy = Mission::new(
        project_id,
        LEGACY_SHARED_OWNER_ID,
        "legacy",
        "goal",
        MissionCadence::Manual,
    );
    store.save_mission(&current).await.unwrap();
    store.save_mission(&legacy).await.unwrap();

    let shared = store.list_shared_missions(project_id).await.unwrap();
    assert_eq!(shared.len(), 2);
}

#[tokio::test]
async fn list_skills_global_returns_only_shared_skills() {
    let store = make_store();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();
    let shared_skill = MemoryDoc::new(
        project_b,
        shared_owner_id(),
        DocType::Skill,
        "shared-skill",
        "",
    );
    let shared_note = MemoryDoc::new(
        project_b,
        shared_owner_id(),
        DocType::Note,
        "shared-note",
        "",
    );
    let alice_skill = MemoryDoc::new(project_a, "alice", DocType::Skill, "alice-skill", "");
    store.save_memory_doc(&shared_skill).await.unwrap();
    store.save_memory_doc(&shared_note).await.unwrap();
    store.save_memory_doc(&alice_skill).await.unwrap();

    let globals = store.list_skills_global().await.unwrap();
    assert_eq!(globals.len(), 1);
    assert_eq!(globals[0].id, shared_skill.id);
}
