//! Tests for admin system prompt (SYSTEM.md in __admin__ scope).
//!
//! When an admin writes SYSTEM.md to the __admin__ scope, all users with
//! `admin_prompt_enabled` set (multi-tenant mode) should see those
//! instructions in their system prompt.
//!
//! These tests verify that:
//! 1. Admin system prompt appears in all users' system prompts
//! 2. No admin prompt when admin_prompt_enabled is false (single-user mode)
//! 3. Admin prompt does not interfere with per-user identity files
//! 4. Empty SYSTEM.md produces no section in the system prompt
#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::workspace::{ADMIN_SCOPE, Workspace, paths};

async fn setup() -> (Arc<dyn Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&db_path).await.expect("create db");
    backend.run_migrations().await.expect("run migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, dir)
}

/// Seed a document into a specific user's workspace scope.
async fn seed(db: &Arc<dyn Database>, user_id: &str, path: &str, content: &str) {
    let ws = Workspace::new_with_db(user_id, db.clone());
    ws.write(path, content)
        .await
        .unwrap_or_else(|e| panic!("Failed to seed {path} for {user_id}: {e}"));
}

// ─── Test 1: Admin system prompt appears for all users ───────────────────

#[tokio::test]
async fn admin_system_prompt_appears_in_user_prompt() {
    let (db, _dir) = setup().await;

    // Admin writes SYSTEM.md to __admin__ scope
    seed(
        &db,
        ADMIN_SCOPE,
        paths::SYSTEM,
        "This is a custom AI assistant for Acme Corp. Always be professional.",
    )
    .await;

    // Create Alice's workspace with admin prompt enabled (multi-tenant mode)
    let ws = Workspace::new_with_db("alice", db.clone()).with_admin_prompt();

    let prompt = ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    assert!(
        prompt.contains("Acme Corp"),
        "Admin system prompt should appear in user's system prompt.\nPrompt:\n{prompt}"
    );
    assert!(
        prompt.contains("## System Instructions"),
        "Admin system prompt should be under '## System Instructions' header.\nPrompt:\n{prompt}"
    );
}

// ─── Test 2: Admin system prompt is NOT shown in single-user mode ────────

#[tokio::test]
async fn admin_system_prompt_hidden_in_single_user_mode() {
    let (db, _dir) = setup().await;

    // Admin writes SYSTEM.md to __admin__ scope
    seed(
        &db,
        ADMIN_SCOPE,
        paths::SYSTEM,
        "This is a custom AI assistant for Acme Corp.",
    )
    .await;

    // Create workspace WITHOUT admin prompt enabled (single-user mode)
    let ws = Workspace::new_with_db("alice", db.clone());

    let prompt = ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    assert!(
        !prompt.contains("Acme Corp"),
        "Admin system prompt must NOT appear when admin_prompt_enabled is false.\nPrompt:\n{prompt}"
    );
}

// ─── Test 3: Admin prompt does not interfere with per-user identity ──────

#[tokio::test]
async fn admin_prompt_coexists_with_user_identity() {
    let (db, _dir) = setup().await;

    // Admin writes SYSTEM.md
    seed(
        &db,
        ADMIN_SCOPE,
        paths::SYSTEM,
        "All agents must follow Acme Corp policies.",
    )
    .await;

    // Alice has her own identity files
    seed(&db, "alice", paths::SOUL, "Alice is kind and creative.").await;
    seed(
        &db,
        "alice",
        paths::USER,
        "You are talking to Alice, a designer.",
    )
    .await;

    let ws = Workspace::new_with_db("alice", db.clone()).with_admin_prompt();

    let prompt = ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    // Both admin prompt and user identity should be present
    assert!(
        prompt.contains("Acme Corp policies"),
        "Admin system prompt should be present.\nPrompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Alice is kind and creative"),
        "User's SOUL.md should still be present.\nPrompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Alice, a designer"),
        "User's USER.md should still be present.\nPrompt:\n{prompt}"
    );

    // Admin prompt should come before identity files
    let admin_pos = prompt
        .find("Acme Corp policies")
        .expect("admin prompt not found");
    let identity_pos = prompt
        .find("Alice is kind")
        .expect("user identity not found");
    assert!(
        admin_pos < identity_pos,
        "Admin system prompt should appear before user identity files.\n\
         Admin position: {admin_pos}, Identity position: {identity_pos}"
    );
}

// ─── Test 4: Empty SYSTEM.md produces no section ─────────────────────────

#[tokio::test]
async fn empty_system_prompt_produces_no_section() {
    let (db, _dir) = setup().await;

    // Admin writes empty SYSTEM.md
    seed(&db, ADMIN_SCOPE, paths::SYSTEM, "").await;

    let ws = Workspace::new_with_db("alice", db.clone()).with_admin_prompt();

    let prompt = ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    assert!(
        !prompt.contains("System Instructions"),
        "Empty SYSTEM.md should not produce a section.\nPrompt:\n{prompt}"
    );
}

// ─── Test 5: Multiple users see the same admin prompt ────────────────────

#[tokio::test]
async fn multiple_users_see_same_admin_prompt() {
    let (db, _dir) = setup().await;

    seed(
        &db,
        ADMIN_SCOPE,
        paths::SYSTEM,
        "Company-wide instruction: always greet users by name.",
    )
    .await;

    // Seed different identity for each user
    seed(&db, "alice", paths::SOUL, "Alice values creativity.").await;
    seed(&db, "bob", paths::SOUL, "Bob values precision.").await;

    let alice_ws = Workspace::new_with_db("alice", db.clone()).with_admin_prompt();
    let bob_ws = Workspace::new_with_db("bob", db.clone()).with_admin_prompt();

    let alice_prompt = alice_ws
        .system_prompt_for_context(false)
        .await
        .expect("alice prompt");
    let bob_prompt = bob_ws
        .system_prompt_for_context(false)
        .await
        .expect("bob prompt");

    // Both see the admin prompt
    assert!(
        alice_prompt.contains("greet users by name"),
        "Alice should see admin prompt.\nPrompt:\n{alice_prompt}"
    );
    assert!(
        bob_prompt.contains("greet users by name"),
        "Bob should see admin prompt.\nPrompt:\n{bob_prompt}"
    );

    // Each sees their own identity
    assert!(
        alice_prompt.contains("Alice values creativity"),
        "Alice should see her own identity.\nPrompt:\n{alice_prompt}"
    );
    assert!(
        !alice_prompt.contains("Bob values precision"),
        "Alice should NOT see Bob's identity.\nPrompt:\n{alice_prompt}"
    );
    assert!(
        bob_prompt.contains("Bob values precision"),
        "Bob should see his own identity.\nPrompt:\n{bob_prompt}"
    );
}

// ─── Test 6: scoped_to_user preserves admin_prompt_enabled ───────────────

#[tokio::test]
async fn scoped_to_user_preserves_admin_prompt() {
    let (db, _dir) = setup().await;

    seed(
        &db,
        ADMIN_SCOPE,
        paths::SYSTEM,
        "Admin prompt: use formal language.",
    )
    .await;
    seed(&db, "bob", paths::SOUL, "Bob is analytical.").await;

    // Create workspace for alice with admin prompt, then scope to bob
    let alice_ws = Workspace::new_with_db("alice", db.clone()).with_admin_prompt();
    let bob_ws = alice_ws.scoped_to_user("bob");

    let prompt = bob_ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    assert!(
        prompt.contains("use formal language"),
        "scoped_to_user should preserve admin_prompt_enabled.\nPrompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Bob is analytical"),
        "Scoped workspace should read Bob's identity.\nPrompt:\n{prompt}"
    );
}
