#![cfg(feature = "libsql")]
//! Integration tests for learning system store traits using file-based libSQL.
//!
//! Tests round-trip CRUD for `UserProfileStore`, `LearningStore`, and `SessionSearchStore`.
//! Uses `LibSqlBackend::new_local()` with `tempfile` for cross-connection state sharing.

use ironclaw::db::{Database, LearningStore, SessionSearchStore, SkillStatus, UserProfileStore};

async fn setup() -> (ironclaw::db::libsql::LibSqlBackend, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");
    let db_path = tmp.path().join("test.db");
    let backend = ironclaw::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .expect("Failed to create libSQL backend");
    backend
        .run_migrations()
        .await
        .expect("Failed to run migrations");
    (backend, tmp)
}

/// Insert a dummy conversation so session_summaries FK is satisfied.
async fn insert_conversation(db: &ironclaw::db::libsql::LibSqlBackend, conv_id: uuid::Uuid) {
    let conn = db.connect().await.unwrap();
    conn.execute(
        "INSERT INTO conversations (id, channel, user_id) VALUES (?1, 'test', 'test')",
        libsql::params![conv_id.to_string()],
    )
    .await
    .expect("Failed to insert dummy conversation");
}

// ==================== UserProfileStore ====================

#[tokio::test]
async fn test_user_profile_upsert_and_get() {
    let (db, _tmp) = setup().await;

    let encrypted_value = b"encrypted-data".to_vec();
    let salt = b"salt-bytes".to_vec();

    let id = db
        .upsert_profile_fact(
            "user1",
            "agent1",
            "preference",
            "lang",
            &encrypted_value,
            &salt,
            0.9,
            "explicit",
        )
        .await
        .expect("upsert failed");

    let facts = db
        .get_profile_facts("user1", "agent1")
        .await
        .expect("get failed");

    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, id);
    assert_eq!(facts[0].fact_key, "lang");
    assert_eq!(facts[0].category, "preference");
    assert_eq!(facts[0].fact_value_encrypted, encrypted_value);
    assert_eq!(facts[0].key_salt, salt);
    assert!((facts[0].confidence - 0.9).abs() < 0.01);
    assert_eq!(facts[0].source, "explicit");
}

#[tokio::test]
async fn test_user_profile_upsert_conflict_updates() {
    let (db, _tmp) = setup().await;

    let id1 = db
        .upsert_profile_fact(
            "user1",
            "agent1",
            "preference",
            "lang",
            b"v1",
            b"s1",
            0.8,
            "inferred",
        )
        .await
        .expect("first upsert");

    let id2 = db
        .upsert_profile_fact(
            "user1",
            "agent1",
            "preference",
            "lang",
            b"v2",
            b"s2",
            0.95,
            "explicit",
        )
        .await
        .expect("second upsert");

    // ON CONFLICT should keep the original id
    assert_eq!(id1, id2);

    let facts = db.get_profile_facts("user1", "agent1").await.unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].fact_value_encrypted, b"v2");
    assert!((facts[0].confidence - 0.95).abs() < 0.01);
    assert_eq!(facts[0].source, "explicit");
}

#[tokio::test]
async fn test_user_profile_get_by_category() {
    let (db, _tmp) = setup().await;

    db.upsert_profile_fact("u", "a", "preference", "k1", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();
    db.upsert_profile_fact("u", "a", "expertise", "k2", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();
    db.upsert_profile_fact("u", "a", "preference", "k3", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();

    let prefs = db
        .get_profile_facts_by_category("u", "a", "preference")
        .await
        .unwrap();
    assert_eq!(prefs.len(), 2);

    let expertise = db
        .get_profile_facts_by_category("u", "a", "expertise")
        .await
        .unwrap();
    assert_eq!(expertise.len(), 1);
}

#[tokio::test]
async fn test_user_profile_delete_fact() {
    let (db, _tmp) = setup().await;

    db.upsert_profile_fact("u", "a", "preference", "key1", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();

    let deleted = db
        .delete_profile_fact("u", "a", "preference", "key1")
        .await
        .unwrap();
    assert!(deleted);

    let deleted_again = db
        .delete_profile_fact("u", "a", "preference", "key1")
        .await
        .unwrap();
    assert!(!deleted_again);

    let facts = db.get_profile_facts("u", "a").await.unwrap();
    assert!(facts.is_empty());
}

#[tokio::test]
async fn test_user_profile_delete_by_category() {
    let (db, _tmp) = setup().await;

    db.upsert_profile_fact("u", "a", "preference", "k1", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();
    db.upsert_profile_fact("u", "a", "preference", "k2", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();
    db.upsert_profile_fact("u", "a", "expertise", "k3", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();

    let deleted = db
        .delete_profile_facts_by_category("u", "a", "preference")
        .await
        .unwrap();
    assert_eq!(deleted, 2);

    let remaining = db.get_profile_facts("u", "a").await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].category, "expertise");
}

#[tokio::test]
async fn test_user_profile_clear_all() {
    let (db, _tmp) = setup().await;

    db.upsert_profile_fact("u", "a", "preference", "k1", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();
    db.upsert_profile_fact("u", "a", "expertise", "k2", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();
    db.upsert_profile_fact("u", "a", "style", "k3", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();

    let deleted = db.clear_profile("u", "a").await.unwrap();
    assert_eq!(deleted, 3);

    let remaining = db.get_profile_facts("u", "a").await.unwrap();
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn test_user_profile_isolation_between_users() {
    let (db, _tmp) = setup().await;

    db.upsert_profile_fact(
        "alice",
        "a",
        "preference",
        "k1",
        b"v",
        b"s",
        0.5,
        "inferred",
    )
    .await
    .unwrap();
    db.upsert_profile_fact("bob", "a", "preference", "k1", b"v", b"s", 0.5, "inferred")
        .await
        .unwrap();

    let alice_facts = db.get_profile_facts("alice", "a").await.unwrap();
    assert_eq!(alice_facts.len(), 1);

    let bob_facts = db.get_profile_facts("bob", "a").await.unwrap();
    assert_eq!(bob_facts.len(), 1);

    db.clear_profile("alice", "a").await.unwrap();
    let bob_after = db.get_profile_facts("bob", "a").await.unwrap();
    assert_eq!(bob_after.len(), 1);
}

#[tokio::test]
async fn test_user_profile_delete_by_category_empty() {
    let (db, _tmp) = setup().await;

    // Deleting from a non-existent category should succeed with 0 affected
    let deleted = db
        .delete_profile_facts_by_category("u", "a", "expertise")
        .await
        .unwrap();
    assert_eq!(deleted, 0);
}

#[tokio::test]
async fn test_user_profile_clear_nonexistent_user() {
    let (db, _tmp) = setup().await;

    // Clearing a non-existent user should succeed with 0 affected
    let deleted = db.clear_profile("nonexistent", "agent").await.unwrap();
    assert_eq!(deleted, 0);
}

// ==================== LearningStore ====================

#[tokio::test]
async fn test_learning_record_and_list() {
    let (db, _tmp) = setup().await;

    let id = db
        .record_synthesized_skill(
            "user1",
            "agent1",
            "auto-abc12345",
            Some("---\nname: test-skill\n---\nDo things"),
            "abc12345hash",
            Some(uuid::Uuid::new_v4()),
            SkillStatus::Pending,
            true,
            85,
        )
        .await
        .expect("record failed");

    let skills = db
        .list_synthesized_skills("user1", "agent1", Some(SkillStatus::Pending))
        .await
        .expect("list failed");

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, id);
    assert_eq!(skills[0].skill_name, "auto-abc12345");
    assert!(skills[0].safety_scan_passed);
    assert_eq!(skills[0].quality_score, 85);
    assert_eq!(skills[0].status, SkillStatus::Pending);
}

#[tokio::test]
async fn test_learning_update_status() {
    let (db, _tmp) = setup().await;

    let id = db
        .record_synthesized_skill(
            "user1",
            "agent1",
            "auto-test",
            Some("content"),
            "hash1",
            None,
            SkillStatus::Pending,
            true,
            90,
        )
        .await
        .unwrap();

    let updated = db
        .update_synthesized_skill_status(id, "user1", SkillStatus::Accepted)
        .await
        .unwrap();
    assert!(updated);

    let skill = db
        .get_synthesized_skill(id, "user1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(skill.status, SkillStatus::Accepted);
    assert!(skill.reviewed_at.is_some());
}

#[tokio::test]
async fn test_learning_status_update_only_from_pending() {
    let (db, _tmp) = setup().await;

    let id = db
        .record_synthesized_skill(
            "user1",
            "agent1",
            "auto-test",
            Some("content"),
            "hash1",
            None,
            SkillStatus::Pending,
            true,
            90,
        )
        .await
        .unwrap();

    db.update_synthesized_skill_status(id, "user1", SkillStatus::Accepted)
        .await
        .unwrap();

    // Try to reject after acceptance — should fail (WHERE status = 'pending')
    let updated = db
        .update_synthesized_skill_status(id, "user1", SkillStatus::Rejected)
        .await
        .unwrap();
    assert!(!updated);
}

#[tokio::test]
async fn test_learning_idor_protection() {
    let (db, _tmp) = setup().await;

    let id = db
        .record_synthesized_skill(
            "user1",
            "agent1",
            "auto-test",
            Some("content"),
            "hash1",
            None,
            SkillStatus::Pending,
            true,
            90,
        )
        .await
        .unwrap();

    // Different user cannot read
    let result = db.get_synthesized_skill(id, "attacker").await.unwrap();
    assert!(result.is_none());

    // Different user cannot update status
    let updated = db
        .update_synthesized_skill_status(id, "attacker", SkillStatus::Accepted)
        .await
        .unwrap();
    assert!(!updated);
}

#[tokio::test]
async fn test_learning_content_hash_dedup() {
    let (db, _tmp) = setup().await;

    db.record_synthesized_skill(
        "user1",
        "agent1",
        "auto-first",
        Some("content"),
        "same_hash",
        None,
        SkillStatus::Pending,
        true,
        80,
    )
    .await
    .unwrap();

    // Same user + same content_hash should fail (unique index)
    let result = db
        .record_synthesized_skill(
            "user1",
            "agent1",
            "auto-second",
            Some("different content"),
            "same_hash",
            None,
            SkillStatus::Pending,
            true,
            85,
        )
        .await;

    assert!(result.is_err(), "duplicate content hash should be rejected");
}

#[tokio::test]
async fn test_learning_list_filter_by_status() {
    let (db, _tmp) = setup().await;

    db.record_synthesized_skill(
        "u",
        "a",
        "s1",
        Some("c"),
        "h1",
        None,
        SkillStatus::Pending,
        true,
        80,
    )
    .await
    .unwrap();

    let id2 = db
        .record_synthesized_skill(
            "u",
            "a",
            "s2",
            Some("c"),
            "h2",
            None,
            SkillStatus::Pending,
            true,
            90,
        )
        .await
        .unwrap();

    db.update_synthesized_skill_status(id2, "u", SkillStatus::Accepted)
        .await
        .unwrap();

    let pending = db
        .list_synthesized_skills("u", "a", Some(SkillStatus::Pending))
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);

    let accepted = db
        .list_synthesized_skills("u", "a", Some(SkillStatus::Accepted))
        .await
        .unwrap();
    assert_eq!(accepted.len(), 1);

    let all = db.list_synthesized_skills("u", "a", None).await.unwrap();
    assert_eq!(all.len(), 2);
}

// ==================== SessionSearchStore ====================

#[tokio::test]
async fn test_session_summary_upsert_and_get() {
    let (db, _tmp) = setup().await;
    let conv_id = uuid::Uuid::new_v4();
    insert_conversation(&db, conv_id).await;

    let id = db
        .upsert_session_summary(
            conv_id,
            "user1",
            "agent1",
            "Discussed Rust error handling patterns",
            &["rust".to_string(), "error-handling".to_string()],
            &["shell".to_string(), "write_file".to_string()],
            15,
            None,
        )
        .await
        .expect("upsert failed");

    let summary = db
        .get_session_summary(conv_id)
        .await
        .expect("get failed")
        .expect("should exist");

    assert_eq!(summary.id, id);
    assert_eq!(summary.conversation_id, conv_id);
    assert_eq!(summary.summary, "Discussed Rust error handling patterns");
    assert_eq!(summary.topics, vec!["rust", "error-handling"]);
    assert_eq!(summary.tool_names, vec!["shell", "write_file"]);
    assert_eq!(summary.message_count, 15);
}

#[tokio::test]
async fn test_session_summary_fts_search() {
    let (db, _tmp) = setup().await;

    let c1 = uuid::Uuid::new_v4();
    let c2 = uuid::Uuid::new_v4();
    insert_conversation(&db, c1).await;
    insert_conversation(&db, c2).await;

    db.upsert_session_summary(
        c1,
        "user1",
        "agent1",
        "Implemented a REST API with authentication",
        &["api".to_string()],
        &["http".to_string()],
        10,
        None,
    )
    .await
    .unwrap();

    db.upsert_session_summary(
        c2,
        "user1",
        "agent1",
        "Fixed database migration script for PostgreSQL",
        &["database".to_string()],
        &["shell".to_string()],
        5,
        None,
    )
    .await
    .unwrap();

    let results = db
        .search_sessions_fts("user1", "database migration", 10)
        .await
        .unwrap();
    assert!(!results.is_empty());
    assert!(results[0].summary.contains("database"));
}

#[tokio::test]
async fn test_session_summary_upsert_updates_existing() {
    let (db, _tmp) = setup().await;
    let conv_id = uuid::Uuid::new_v4();
    insert_conversation(&db, conv_id).await;

    let id1 = db
        .upsert_session_summary(
            conv_id,
            "user1",
            "agent1",
            "Initial summary",
            &[],
            &[],
            5,
            None,
        )
        .await
        .unwrap();

    let id2 = db
        .upsert_session_summary(
            conv_id,
            "user1",
            "agent1",
            "Updated summary with more detail",
            &["topic".to_string()],
            &["tool".to_string()],
            20,
            None,
        )
        .await
        .unwrap();

    assert_eq!(id1, id2);

    let summary = db.get_session_summary(conv_id).await.unwrap().unwrap();
    assert_eq!(summary.summary, "Updated summary with more detail");
    assert_eq!(summary.message_count, 20);
}

#[tokio::test]
async fn test_session_summary_user_isolation() {
    let (db, _tmp) = setup().await;

    let c1 = uuid::Uuid::new_v4();
    let c2 = uuid::Uuid::new_v4();
    insert_conversation(&db, c1).await;
    insert_conversation(&db, c2).await;

    db.upsert_session_summary(
        c1,
        "alice",
        "a",
        "Alice private session about passwords",
        &[],
        &[],
        3,
        None,
    )
    .await
    .unwrap();

    db.upsert_session_summary(
        c2,
        "bob",
        "a",
        "Bob public session about databases",
        &[],
        &[],
        3,
        None,
    )
    .await
    .unwrap();

    let alice_results = db
        .search_sessions_fts("alice", "session", 10)
        .await
        .unwrap();
    let bob_results = db.search_sessions_fts("bob", "session", 10).await.unwrap();

    for r in &alice_results {
        assert_eq!(r.user_id, "alice");
    }
    for r in &bob_results {
        assert_eq!(r.user_id, "bob");
    }
}
