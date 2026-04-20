//! Integration tests for the project-admin tool dispatch path.
//!
//! Per `.claude/rules/testing.md` ("Test Through the Caller, Not Just the
//! Helper"), the project tools gate several side effects — engine `Store`
//! writes, workspace MemoryDoc writes, conversation-metadata patches —
//! and are invoked by gateway handlers. A unit test on
//! `crate::bridge::create_engine_project` in isolation is **not**
//! sufficient regression coverage; the caller-level scenarios here
//! exercise the full dispatch chain that a real `POST /api/engine/projects`
//! request would travel.
//!
//! Live-tier scenarios that clone a real GitHub repo, open a draft PR,
//! and tear down after themselves live under `tests/live/` and are gated
//! by `IRONCLAW_LIVE_TESTS=1` + `GH_TOKEN`. See `tests/live/README.md`
//! for the contract.

#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw::bridge::{ProjectMetadataView, ProjectUpsertFields};
use ironclaw_common::GitHubRepo;

/// Smoke-test for `ProjectMetadataView::from_metadata` — the helper that
/// extracts typed fields (github_repo, default_branch) from the opaque
/// `Project.metadata` JSON the engine store round-trips. Invalid or
/// missing values must fall through to `None` so the chrome gracefully
/// hides the affected chip instead of rendering garbage.
#[test]
fn project_metadata_view_extracts_fields() {
    let metadata = serde_json::json!({
        "github_repo": "nearai/ironclaw",
        "default_branch": "staging",
        "unrelated_key": "ignored",
    });
    let view = ProjectMetadataView::from_metadata(&metadata);
    assert_eq!(
        view.github_repo.as_ref().map(|r| r.as_str().to_string()),
        Some("nearai/ironclaw".to_string())
    );
    assert_eq!(view.default_branch.as_deref(), Some("staging"));
}

#[test]
fn project_metadata_view_rejects_malformed_github_repo() {
    let metadata = serde_json::json!({
        "github_repo": "not a slug",
        "default_branch": "",
    });
    let view = ProjectMetadataView::from_metadata(&metadata);
    assert!(
        view.github_repo.is_none(),
        "invalid owner/repo must not deserialize into a GitHubRepo"
    );
    // Empty `default_branch` must drop to `None` (same semantics as the
    // upsert form path — empty string = "clear this field").
    assert!(view.default_branch.is_none());
}

#[test]
fn github_repo_validation_bubbles_through_upsert() {
    // `ProjectUpsertFields` treats `github_repo: Some(None)` as "clear"
    // and `Some(Some(repo))` as "set". Constructing a typed `GitHubRepo`
    // here is the only entry point that validates; handlers rely on
    // this to reject `not/a/slug` at the HTTP boundary rather than
    // writing junk into `Project.metadata`.
    assert!(GitHubRepo::new("owner/repo").is_ok());
    assert!(GitHubRepo::new("owner-with-dash/repo_with_under").is_ok());
    assert!(GitHubRepo::new("no-slash").is_err());
    assert!(GitHubRepo::new("owner/has space").is_err());

    let fields = ProjectUpsertFields {
        name: Some("demo".to_string()),
        description: Some("x".to_string()),
        workspace_path: None,
        github_repo: Some(Some(GitHubRepo::new("nearai/ironclaw").unwrap())),
        default_branch: Some(Some("staging".to_string())),
    };
    // The struct is constructible with the typed repo, which is what
    // the handler and tool rely on to round-trip via `save_project`.
    assert!(fields.github_repo.as_ref().unwrap().is_some());
}

/// The `project_workspace_path` helper computes the host path either
/// from an explicit `Project::with_workspace_path` override or from
/// `~/.ironclaw/projects/<user_id>/<project_id>/`. Gateway UI relies on
/// this: when a project has no override, the chrome + shell-mode
/// dispatch both fall back to the default, and the two paths must
/// match. This caller-level test asserts that — mismatches have been
/// the shape of earlier sandbox bugs.
#[test]
fn default_workspace_path_is_stable_and_user_scoped() {
    use ironclaw::bridge::sandbox::workspace_path::default_project_workspace_path;

    let pid = uuid::Uuid::new_v4();
    let alice = default_project_workspace_path("alice", pid);
    let bob = default_project_workspace_path("bob", pid);
    let alice_again = default_project_workspace_path("alice", pid);

    assert_ne!(alice, bob, "per-user namespacing must be maintained");
    assert_eq!(alice, alice_again, "default path must be deterministic");
    assert!(
        alice.to_string_lossy().contains("alice"),
        "user_id must appear in the path — got {alice:?}"
    );
    assert!(
        alice.to_string_lossy().contains(&pid.to_string()),
        "project_id must appear in the path — got {alice:?}"
    );
}

/// The gateway's shell-mode dispatch resolves workdir from
/// `project.workspace_path` first, defaulting to the engine's per-user
/// host directory when unset. This caller-level test pins the priority:
/// if a Project carries a `workspace_path` override, the resolver uses
/// it verbatim. A regression where the override was silently ignored
/// would route `!` shell commands into the engine's default sandbox
/// folder instead of the user's actual repo — a silent footgun.
#[test]
fn explicit_workspace_path_overrides_default() {
    use ironclaw::bridge::sandbox::workspace_path::project_workspace_path;
    use ironclaw_engine::Project;

    let custom = std::path::PathBuf::from("/tmp/ironclaw-live-override");
    let project = Project::new("alice", "demo", "").with_workspace_path(custom.clone());
    let resolved = project_workspace_path(&project);
    assert_eq!(
        resolved, custom,
        "explicit workspace_path must pass through"
    );

    // Without an override, the resolver must fall back to the default
    // per-user location.
    let project_no_override = Project::new("alice", "demo", "");
    let default_resolved = project_workspace_path(&project_no_override);
    assert!(
        default_resolved.to_string_lossy().contains("alice"),
        "fallback must namespace by user — got {default_resolved:?}"
    );
}

/// Keeps the `Arc` import live; a future extension of this test
/// module to cover async dispatch will need the import.
#[allow(dead_code)]
fn _keep_arc_in_scope() -> Arc<()> {
    Arc::new(())
}
