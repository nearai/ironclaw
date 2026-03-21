//! Tests for identity file scope isolation in multi-scope workspaces.
//!
//! When a workspace has multiple read scopes (e.g., Andrew can read from
//! "andrew", "grace", "household"), identity files (SOUL.md, USER.md,
//! IDENTITY.md, AGENTS.md) must ONLY come from the primary scope.
//!
//! Multi-scope reads are designed for memory sharing (MEMORY.md, daily logs),
//! not identity inheritance. Silently inheriting identity from another scope
//! is a correctness and security issue — the agent would present itself as
//! the wrong user.
//!
//! These tests verify that:
//! 1. Identity files are read from primary scope only
//! 2. If the primary scope's identity file is missing, it's absent from the
//!    system prompt — never falls back to another scope
//! 3. Memory files (MEMORY.md) still benefit from multi-scope reads
#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::Database;
use ironclaw::workspace::{paths, Workspace};

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
    ws.write(path, content).await.unwrap_or_else(|e| {
        panic!("Failed to seed {path} for {user_id}: {e}")
    });
}

// ─── Test 1: Primary scope identity appears in system prompt ───────────

#[tokio::test]
async fn system_prompt_uses_primary_scope_identity() {
    let (db, _dir) = setup().await;

    // Seed Alice's identity files in her own scope
    seed(&db, "alice", paths::SOUL, "Alice is kind and curious.").await;
    seed(&db, "alice", paths::USER, "You are talking to Alice, a software engineer.").await;

    // Seed Bob's identity files in his scope
    seed(&db, "bob", paths::SOUL, "Bob is analytical and precise.").await;
    seed(&db, "bob", paths::USER, "You are talking to Bob, a marine biologist.").await;

    // Create Alice's workspace WITH multi-scope reads including Bob
    let ws = Workspace::new_with_db("alice", db.clone())
        .with_additional_read_scopes(vec!["bob".to_string()]);

    let prompt = ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    // Alice's identity must appear
    assert!(
        prompt.contains("Alice is kind and curious"),
        "Primary scope SOUL.md should appear in system prompt.\nPrompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Alice, a software engineer"),
        "Primary scope USER.md should appear in system prompt.\nPrompt:\n{prompt}"
    );

    // Bob's identity must NOT appear
    assert!(
        !prompt.contains("Bob is analytical"),
        "Secondary scope SOUL.md must NOT appear in system prompt.\nPrompt:\n{prompt}"
    );
    assert!(
        !prompt.contains("Bob, a marine biologist"),
        "Secondary scope USER.md must NOT appear in system prompt.\nPrompt:\n{prompt}"
    );
}

// ─── Test 2: Missing primary identity does NOT fall back to other scope ─

#[tokio::test]
async fn missing_primary_identity_does_not_fallback_to_other_scope() {
    let (db, _dir) = setup().await;

    // Only seed Bob's identity — Alice has no identity files
    seed(&db, "bob", paths::SOUL, "Bob is analytical and precise.").await;
    seed(&db, "bob", paths::USER, "You are talking to Bob, a marine biologist.").await;

    // Create Alice's workspace with multi-scope reads including Bob
    let ws = Workspace::new_with_db("alice", db.clone())
        .with_additional_read_scopes(vec!["bob".to_string()]);

    let prompt = ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    // Bob's identity must NOT appear — Alice's missing identity should stay missing,
    // not silently inherit from Bob's scope
    assert!(
        !prompt.contains("Bob"),
        "When primary scope identity is missing, must NOT fall back to secondary scope.\n\
         This would cause the agent to present itself as the wrong user.\nPrompt:\n{prompt}"
    );
}

// ─── Test 3: MEMORY.md still benefits from multi-scope reads ────────────

#[tokio::test]
async fn memory_files_still_use_multi_scope_reads() {
    let (db, _dir) = setup().await;

    // Seed shared memory in the "shared" scope (not Alice's primary)
    seed(&db, "shared", paths::MEMORY, "Shared grocery list: milk, eggs, bread.").await;

    // Create Alice's workspace with read access to shared scope
    let ws = Workspace::new_with_db("alice", db.clone())
        .with_additional_read_scopes(vec!["shared".to_string()]);

    let prompt = ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    // Shared memory SHOULD appear — multi-scope reads are correct for memory
    assert!(
        prompt.contains("grocery list"),
        "MEMORY.md should still use multi-scope reads.\nPrompt:\n{prompt}"
    );
}

// ─── Test 4: All identity files are scope-isolated ──────────────────────

#[tokio::test]
async fn all_identity_files_are_scope_isolated() {
    let (db, _dir) = setup().await;

    // Seed identity files ONLY in the "other" scope, not in Alice's
    seed(&db, "other", paths::AGENTS, "You are Other's agent.").await;
    seed(&db, "other", paths::SOUL, "Other's soul values.").await;
    seed(&db, "other", paths::USER, "You are talking to Other.").await;
    seed(&db, "other", paths::IDENTITY, "Other's identity.").await;

    // Also seed BOOTSTRAP.md and TOOLS.md in other scope
    seed(&db, "other", "BOOTSTRAP.md", "Other's bootstrap.").await;
    seed(&db, "other", "TOOLS.md", "Other's tool notes.").await;

    // Create Alice's workspace with read access to "other"
    let ws = Workspace::new_with_db("alice", db.clone())
        .with_additional_read_scopes(vec!["other".to_string()]);

    let prompt = ws
        .system_prompt_for_context(false)
        .await
        .expect("system_prompt_for_context failed");

    // None of Other's identity/config files should appear
    assert!(
        !prompt.contains("Other"),
        "No identity or config files from secondary scope should appear.\n\
         Every identity file (AGENTS.md, SOUL.md, USER.md, IDENTITY.md, \
         BOOTSTRAP.md, TOOLS.md) must read from primary scope only.\nPrompt:\n{prompt}"
    );
}
