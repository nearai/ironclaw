//! Direct regression tests for `Workspace::scoped_to_user()` rebinding.
//!
//! Issue: <https://github.com/nearai/ironclaw/issues/1652>
//!
//! `scoped_to_user()` rebinds a workspace to a different primary user while
//! preserving shared read scopes. These 7 tests verify the rebinding contracts
//! directly, rather than relying on high-level system prompt tests.
//!
//! Related indirect coverage:
//! - `tests/identity_scope_isolation.rs` — identity file scope isolation via system prompt
//! - `tests/multi_tenant_system_prompt.rs` — per-user prompt content
//! - `tests/multi_scope_functional.rs` — multi-scope read/write/search
#![cfg(feature = "libsql")]

use std::collections::HashSet;
use std::sync::Arc;

use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::workspace::layer::{LayerSensitivity, MemoryLayer};
use ironclaw::workspace::{Workspace, paths};

// ─── Helpers ──────────────────────────────────────────────────────────────

async fn setup() -> (Arc<dyn Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&db_path).await.expect("create db");
    backend.run_migrations().await.expect("run migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, dir)
}

async fn seed(db: &Arc<dyn Database>, user_id: &str, path: &str, content: &str) {
    let ws = Workspace::new_with_db(user_id, db.clone());
    ws.write(path, content)
        .await
        .unwrap_or_else(|e| panic!("Failed to seed {path} for {user_id}: {e}"));
}

fn assert_no_duplicates(scopes: &[String]) {
    let set: HashSet<&String> = scopes.iter().collect();
    assert_eq!(
        set.len(),
        scopes.len(),
        "read_user_ids contains duplicates: {scopes:?}"
    );
}

fn secondary_scopes(ws: &Workspace) -> HashSet<String> {
    ws.read_user_ids()
        .iter()
        .skip(1) // skip primary at index 0
        .cloned()
        .collect()
}

// ─── Test 1: Primary user ID changes ─────────────────────────────────────

#[tokio::test]
async fn rebind_changes_primary_user_id() {
    let (db, _dir) = setup().await;

    let ws = Workspace::new_with_db("alice", db);
    let rebound = ws.scoped_to_user("bob");

    // Original workspace is unchanged (clone, not mutate).
    assert_eq!(ws.user_id(), "alice");

    // Rebound workspace reflects the new primary.
    assert_eq!(rebound.user_id(), "bob");
    assert_eq!(
        rebound.read_user_ids().first().map(String::as_str),
        Some("bob"),
        "primary must be the first element in read_user_ids"
    );
}

// ─── Test 2: Private layers rescoped, shared layers untouched ────────────

#[tokio::test]
async fn rebind_rescopes_private_layers_without_mutating_shared_layers() {
    let (db, _dir) = setup().await;

    let ws = Workspace::new_with_db("alice", db).with_memory_layers(vec![
        MemoryLayer {
            name: "private".to_string(),
            scope: "alice".to_string(),
            writable: true,
            sensitivity: LayerSensitivity::Private,
        },
        MemoryLayer {
            name: "team".to_string(),
            scope: "team".to_string(),
            writable: false,
            sensitivity: LayerSensitivity::Shared,
        },
    ]);

    let rebound = ws.scoped_to_user("bob");

    // Private layer scope rewritten from alice → bob.
    let private =
        MemoryLayer::find(rebound.memory_layers(), "private").expect("private layer must exist");
    assert_eq!(
        private.scope, "bob",
        "private layer must be rescoped to bob"
    );

    // Shared layer scope unchanged.
    let team = MemoryLayer::find(rebound.memory_layers(), "team").expect("team layer must exist");
    assert_eq!(team.scope, "team", "shared layer must not be mutated");

    // Derived read_user_ids: no residual "alice", includes "bob" and "team".
    let ids = rebound.read_user_ids();
    assert!(
        !ids.contains(&"alice".to_string()),
        "old primary must not remain in read_user_ids"
    );
    assert!(ids.contains(&"bob".to_string()));
    assert!(ids.contains(&"team".to_string()));
    assert_no_duplicates(ids);
}

// ─── Test 3: Secondary read scopes preserved, old primary removed ────────

#[tokio::test]
async fn rebind_preserves_secondary_read_scopes_and_removes_old_primary() {
    let (db, _dir) = setup().await;

    let ws = Workspace::new_with_db("alice", db)
        .with_additional_read_scopes(vec!["shared".to_string(), "reports".to_string()]);

    let rebound = ws.scoped_to_user("bob");
    let ids = rebound.read_user_ids();

    assert_eq!(ids[0], "bob", "new primary must be first");
    assert!(
        !ids.contains(&"alice".to_string()),
        "old primary must be removed"
    );
    assert_no_duplicates(ids);

    // Secondary scope set must be exactly {"shared", "reports"}.
    let expected: HashSet<String> = ["shared", "reports"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        secondary_scopes(&rebound),
        expected,
        "secondary scopes must be preserved exactly"
    );
}

// ─── Test 4: Identity reads pinned to new primary ────────────────────────

#[tokio::test]
async fn rebind_keeps_identity_reads_pinned_to_new_primary() {
    let (db, _dir) = setup().await;

    seed(&db, "alice", paths::IDENTITY, "I am Alice").await;
    seed(&db, "bob", paths::IDENTITY, "I am Bob").await;
    seed(&db, "shared", paths::IDENTITY, "I am Shared").await;

    let ws = Workspace::new_with_db("alice", db)
        .with_additional_read_scopes(vec!["bob".to_string(), "shared".to_string()]);
    let rebound = ws.scoped_to_user("bob");

    // read_primary — explicit primary-only API.
    let doc1 = rebound
        .read_primary(paths::IDENTITY)
        .await
        .expect("read_primary should succeed");
    assert_eq!(
        doc1.content, "I am Bob",
        "read_primary must return exactly the new primary's identity"
    );

    // read — identity paths get special primary-only treatment (mod.rs:665).
    let doc2 = rebound
        .read(paths::IDENTITY)
        .await
        .expect("read should succeed");
    assert_eq!(
        doc2.content, "I am Bob",
        "read must return exactly the new primary's identity for identity paths"
    );
}

// ─── Test 5: Non-identity reads still span shared scopes ─────────────────

#[tokio::test]
async fn rebind_preserves_shared_non_identity_reads_after_old_primary_removal() {
    let (db, _dir) = setup().await;

    // Same path, different content — proves old primary removal + shared preservation.
    seed(&db, "alice", "notes/team.md", "Alice version").await;
    seed(&db, "shared", "notes/team.md", "Shared version").await;

    let ws =
        Workspace::new_with_db("alice", db).with_additional_read_scopes(vec!["shared".to_string()]);
    let rebound = ws.scoped_to_user("bob");

    // Structural precondition: old primary removed, shared preserved.
    assert!(
        !rebound.read_user_ids().contains(&"alice".to_string()),
        "old primary must not be in read_user_ids"
    );
    assert!(
        rebound.read_user_ids().contains(&"shared".to_string()),
        "shared scope must be preserved"
    );

    let doc = rebound
        .read("notes/team.md")
        .await
        .expect("read should succeed from shared scope");
    assert_eq!(
        doc.content, "Shared version",
        "must read exactly the shared scope content, not old primary"
    );
}

// ─── Test 6: Same-user rebind preserves bootstrap flags ──────────────────

#[tokio::test]
async fn rebind_same_user_preserves_bootstrap_flags() {
    // Fresh DB — seed_if_empty() needs a truly empty workspace.
    let (db, _dir) = setup().await;

    let ws = Workspace::new_with_db("alice", db);
    ws.seed_if_empty()
        .await
        .expect("seed_if_empty should succeed");
    ws.mark_bootstrap_completed();

    let same = ws.scoped_to_user("alice");
    assert!(
        same.take_bootstrap_pending(),
        "bootstrap_pending must be preserved on same-user rebind"
    );
    assert!(
        same.is_bootstrap_completed(),
        "bootstrap_completed must be preserved on same-user rebind"
    );
}

// ─── Test 7: Different-user rebind resets pending but preserves completed ──

#[tokio::test]
async fn rebind_different_user_resets_bootstrap_pending_but_preserves_completed() {
    // Fresh DB independent from Test 6 — seed_if_empty() won't fire on a
    // non-fresh workspace, so we cannot reuse a DB where "alice" was already seeded.
    let (db, _dir) = setup().await;

    let ws = Workspace::new_with_db("alice", db);
    ws.seed_if_empty()
        .await
        .expect("seed_if_empty should succeed");
    ws.mark_bootstrap_completed();

    // Precondition: prove the source workspace has both flags set before rebinding.
    // Use a same-user clone to observe without consuming the original's atomic state.
    let probe = ws.scoped_to_user("alice");
    assert!(
        probe.take_bootstrap_pending(),
        "precondition: source workspace must have bootstrap_pending = true"
    );
    assert!(
        probe.is_bootstrap_completed(),
        "precondition: source workspace must have bootstrap_completed = true"
    );

    let different = ws.scoped_to_user("bob");
    assert!(
        !different.take_bootstrap_pending(),
        "bootstrap_pending must be reset on different-user rebind"
    );
    assert!(
        different.is_bootstrap_completed(),
        "bootstrap_completed must be preserved on different-user rebind"
    );
}
